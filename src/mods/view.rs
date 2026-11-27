//! The Mods page's frame: the page it shows, its shared strip and header.

use super::*;
use crate::widgets::button::{button, Look, Size, Style};

impl Launcher {
    /// The Mods screen for `self.ui.page`: Discover, Installed or a mod's page.
    pub(crate) fn mods(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.mods_drain();
        self.mods_refresh();
        self.refresh_nightly_scope();
        if self.ui.page == Page::Packs {
            return self.pack_page(window, cx);
        }
        let m = &mut self.mods_state;
        let find_focus = m
            .find_focus
            .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        let have_focus = m
            .have_focus
            .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        match self.ui.page {
            Page::Project if self.mods_state.nightly.collection.is_some() => {
                self.source_collection(cx)
            }
            Page::Project
                if self.mods_state.open.is_none() && self.mods_state.nightly.open.is_some() =>
            {
                self.nightly_project(cx)
            }
            Page::Project => {
                let scope = self.mods_page_scope();
                if self
                    .mods_state
                    .open
                    .as_ref()
                    .is_some_and(|open| open.scope != scope)
                {
                    let open = self.mods_state.open.take().expect("different open scope");
                    if open.installed.is_some()
                        && (scope.profile != open.scope.profile || scope.home != open.scope.home)
                    {
                        self.ui.page = Page::Installed;
                        return self.installed(window, &have_focus, cx);
                    }
                    if open.v2 {
                        self.mods_load_v2_page(open.id, open.from);
                    } else {
                        self.mods_load_page(open.id, open.from, open.installed);
                    }
                    if let Some(now) = self.mods_state.open.as_mut() {
                        now.parent = open.parent;
                    }
                }
                let listing = self.mods_state.open.as_ref().and_then(|open| {
                    let Some(InstalledProject::Local(file)) = &open.installed else {
                        return None;
                    };
                    self.installed_source_project(&open.id, file).cloned()
                });
                if let Some(source) = listing {
                    if self
                        .mods_state
                        .nightly
                        .open
                        .as_ref()
                        .is_none_or(|open| open.source != source)
                    {
                        self.inspect_nightly_from(source, false, Page::Installed);
                    }
                    self.nightly_project(cx)
                } else {
                    self.mods_state.nightly.open = None;
                    self.project(cx)
                }
            }
            Page::Installed => self.installed(window, &have_focus, cx),
            Page::PackFind => self.pack_find(window, &find_focus, cx),
            Page::PackProject => self.pack_project(cx),
            _ => self.discover(window, &find_focus, cx),
        }
    }

    /// The width a page's own content has: the window less its edges, the sidebar (open, closed or
    /// on its way), the gap beside it, the pane's border and the page's margins.
    pub(crate) fn content_width(&self) -> f32 {
        let edges = 2. * crate::tokens::FRAME_INSET + 12.;
        self.window_width - edges - crate::rail::rail_width(self.rail_open()) - 2. - 48.
    }

    pub(crate) fn open_url(url: &str, cx: &mut Context<Self>) {
        if url.starts_with("https://") {
            cx.open_url(url);
        }
    }

    /// The line shown above either list: the last result. Work under way is the status strip's,
    /// on every page, so this page draws no second bar for it.
    pub(super) fn strip(&self, cx: &mut Context<Self>) -> Option<Div> {
        use ui::{c, muted};
        let notice = self.notice()?;
        let (text, color) = match notice {
            Ok(t) => (t, palette().text),
            Err(t) => (t, palette().warning),
        };
        Some(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(tokens::LABEL))
                        .text_color(c(color))
                        .child(text),
                )
                .child(
                    muted(tr("Dismiss"))
                        .id("alt-mods-dismiss")
                        .reach()
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.dismiss_notice();
                            cx.notify();
                        })),
                ),
        )
    }

    /// The Installed pages' head: the title with the way to Discover, the pack kinds as tabs on
    /// Packs, then the search field (with Add from source on Mods).
    pub(super) fn mods_header(
        &mut self,
        focus: &FocusHandle,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let packs = matches!(self.ui.page, Page::Packs | Page::PackFind);
        let field = self.installed_search(packs, focus, window, cx);
        let discover_page = if packs {
            Page::PackFind
        } else {
            Page::Discover
        };
        let to_discover = button(
            "alt-installed-discover",
            tr("Discover"),
            Look::new(Style::Ghost, Size::Medium).icon("compass-01"),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.choose_view(discover_page);
            cx.notify();
        }));
        let toolbar = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(field)
            .when(!packs, |d| d.child(self.add_from_source_button(cx)));
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(crate::widgets::nav::page_head(
                tr("Installed"),
                Some(to_discover.into_any_element()),
            ))
            .when(packs, |d| d.child(self.collection_tabs(cx)))
            .child(toolbar)
    }

    /// The search field over what is installed: Mods, or the shelf of the pack kind on show.
    fn installed_search(
        &self,
        packs: bool,
        focus: &FocusHandle,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        use ui::field_theme;
        let t = field_theme(self.theme);
        let focused = focus.is_focused(window);
        let placeholder: SharedString = if packs {
            match self.mods_state.shelf {
                Shelf::Resource => tr("Search your resource packs…"),
                Shelf::Shader => tr("Search your shader packs…"),
                Shelf::Data => tr("Search your data packs…"),
            }
        } else {
            tr("Search your mods…")
        }
        .into();
        let field = crate::widgets::field::field_box(focused, false)
            .id("alt-mods-have")
            .track_focus(focus)
            .seen()
            .flex_1()
            .min_w(px(120.))
            .h(px(tokens::FIELD))
            .px(px(16.))
            .rounded(crate::tokens::r(crate::tokens::R_CARD))
            .text_size(px(tokens::BODY))
            .cursor_text()
            .child(line_edit::content_with(
                &self.mods_state.have,
                &t,
                focused,
                false,
                false,
                placeholder,
            ))
            .on_click({
                let focus = focus.clone();
                cx.listener(move |_, _: &ClickEvent, window, cx| {
                    window.focus(&focus, cx);
                    cx.notify();
                })
            })
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                let m = &mut this.mods_state;
                match line_edit::handle(&mut m.have, ev, cx) {
                    Key::Escape => window.blur(cx),
                    Key::Pass => return,
                    Key::Edited => {
                        m.row_filter.text = m.have.text().to_owned();
                        // A new filter is a new scope: nothing stays selected out of sight.
                        m.selection.clear();
                        m.anchor = None;
                    }
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        line_edit::mouse(field, cx, |this: &mut Launcher| &mut this.mods_state.have)
    }
}
