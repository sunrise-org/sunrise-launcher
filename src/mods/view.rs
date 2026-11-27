//! The Mods page's frame: the page it shows, its shared strip and header.

use super::*;

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

    /// Title, Discover/Installed switch and the search field, shared by both lists.
    pub(super) fn mods_header(
        &mut self,
        installed: bool,
        focus: &FocusHandle,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        use ui::field_theme;
        let t = field_theme(self.theme);
        let focused = focus.is_focused(window);
        let packs = matches!(self.ui.page, Page::Packs | Page::PackFind);
        let placeholder: SharedString = match (packs, installed) {
            (false, false) => tr("Search mods…"),
            (false, true) => tr("Search your mods…"),
            (true, false) => match self.mods_state.packs.scope {
                pack_discover::Scope::Resource => tr("Search resource packs…"),
                pack_discover::Scope::Shader => tr("Search shader packs…"),
                pack_discover::Scope::Data => tr("Search data packs…"),
            },
            (true, true) => match self.mods_state.shelf {
                Shelf::Resource => tr("Search your resource packs…"),
                Shelf::Shader => tr("Search your shader packs…"),
                Shelf::Data => tr("Search your data packs…"),
            },
        }
        .into();
        let edit = if installed {
            &self.mods_state.have
        } else {
            &self.mods_state.find
        };
        let field = crate::widgets::field::field_box(focused, false)
            .id(if installed {
                "alt-mods-have"
            } else {
                "alt-mods-find"
            })
            .track_focus(focus)
            .seen()
            .flex_1()
            .min_w(px(120.))
            .h(px(tokens::ACTION))
            .px(px(16.))
            .rounded(crate::tokens::r(crate::tokens::R_CARD))
            .text_size(px(tokens::BODY))
            .cursor_text()
            .child(line_edit::content_with(
                edit,
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
                let packs_scope = this.ui.page == Page::PackFind;
                let m = &mut this.mods_state;
                let edit = if installed { &mut m.have } else { &mut m.find };
                match line_edit::handle(edit, ev, cx) {
                    Key::Escape => window.blur(cx),
                    Key::Pass => return,
                    Key::Enter if !installed && packs_scope => {
                        this.pack_search();
                    }
                    Key::Edited => {
                        let text = edit.text().to_owned();
                        if installed {
                            m.row_filter.text = text;
                            // A new filter is a new scope: nothing stays selected out of sight.
                            m.selection.clear();
                            m.anchor = None;
                        } else {
                            m.filters.text = text;
                            m.limit = 0;
                        }
                    }
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let field = if installed {
            line_edit::mouse(field, cx, |this: &mut Launcher| &mut this.mods_state.have)
        } else {
            line_edit::mouse(field, cx, |this: &mut Launcher| &mut this.mods_state.find)
        };
        let this = cx.entity();
        let installed_page = if packs { Page::Packs } else { Page::Installed };
        let discover_page = if packs {
            Page::PackFind
        } else {
            Page::Discover
        };
        let sub_tabs = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_x(px(16.))
            .gap_y(px(8.))
            .border_b_1()
            .border_color(palette().rule())
            .child(
                crate::widgets::nav::tabs(
                    "alt-mods-tabs",
                    &[
                        (installed_page, tr("Installed").into()),
                        (discover_page, tr("Discover").into()),
                    ],
                    if installed {
                        installed_page
                    } else {
                        discover_page
                    },
                    move |page, _, cx| {
                        this.update(cx, |view, cx| {
                            Launcher::choose_view(view, page);
                            cx.notify();
                        });
                    },
                )
                .seen(),
            );
        let toolbar = if installed && !packs {
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.))
                .child(field)
                .child(self.add_from_source_button(cx))
                .into_any_element()
        } else {
            div().flex().items_center().child(field).into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(sub_tabs)
            .child(toolbar)
    }
}
