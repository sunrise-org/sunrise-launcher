//! Discover's header and filter row: the title, the kind tabs, the search field, the profile
//! picker, the category chips and the sort menu, and the count line above the list.
use super::*;
use crate::widgets::field::select_button;
use std::rc::Rc;

/// The kinds Discover browses: mods on Discover itself, and the three pack kinds on Packs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::mods) enum KindTab {
    Mods,
    Resource,
    Shader,
    Data,
}

impl KindTab {
    const ALL: [KindTab; 4] = [
        KindTab::Mods,
        KindTab::Resource,
        KindTab::Shader,
        KindTab::Data,
    ];

    fn label(self) -> &'static str {
        match self {
            KindTab::Mods => tr("Mods"),
            KindTab::Resource => tr("Resource packs"),
            KindTab::Shader => tr("Shader packs"),
            KindTab::Data => tr("Data packs"),
        }
    }

    pub(in crate::mods) fn from_scope(scope: pack_discover::Scope) -> KindTab {
        match scope {
            pack_discover::Scope::Resource => KindTab::Resource,
            pack_discover::Scope::Shader => KindTab::Shader,
            pack_discover::Scope::Data => KindTab::Data,
        }
    }

    fn search_placeholder(self) -> &'static str {
        match self {
            KindTab::Mods => tr("Search mods…"),
            KindTab::Resource => tr("Search resource packs…"),
            KindTab::Shader => tr("Search shader packs…"),
            KindTab::Data => tr("Search data packs…"),
        }
    }
}

/// How many category chips sit in the row before the rest move behind "more".
const CATEGORY_CHIPS: usize = 5;

impl Launcher {
    /// Title, kind tabs and search, then the filter row; the page's own content follows.
    pub(in crate::mods) fn discover_shell(
        &mut self,
        tab: KindTab,
        window: &mut Window,
        focus: &FocusHandle,
        cx: &mut Context<Self>,
    ) -> Div {
        let header = self.discover_header(tab, focus, window, cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(24.))
            .text_color(c(palette().text))
            .child(header)
            .children(self.strip(cx))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.mods_state.menu.take().is_some() {
                        cx.notify();
                    }
                }),
            )
    }

    /// The title row (the title, the add-from-link and profile controls) over the kind tabs and the
    /// search field.
    fn discover_header(
        &self,
        tab: KindTab,
        focus: &FocusHandle,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let title_row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .child(
                div()
                    .text_size(px(tokens::HEADING))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(tr("Discover")),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_end()
                    .gap(px(12.))
                    .child(self.add_from_source_button(cx))
                    .child(self.profile_picker(cx))
                    .child(self.installed_link(tab, cx)),
            );
        let tabs_row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_x(px(16.))
            .gap_y(px(8.))
            .child(self.kind_tabs(tab, cx))
            .child(self.discover_search(tab, focus, window, cx));
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(title_row)
            .child(tabs_row)
    }

    fn kind_tabs(&self, tab: KindTab, cx: &mut Context<Self>) -> Stateful<Div> {
        let options: Vec<(KindTab, SharedString)> = KindTab::ALL
            .into_iter()
            .map(|kind| (kind, kind.label().into()))
            .collect();
        let this = cx.entity();
        crate::widgets::nav::tabs("alt-mods-tabs", &options, tab, move |kind, _, cx| {
            this.update(cx, |view, cx| {
                view.pick_kind(kind);
                cx.notify();
            });
        })
    }

    /// Opens the kind a tab names: Mods on Discover, a pack kind on Packs' Discover.
    fn pick_kind(&mut self, kind: KindTab) {
        let shelf = match kind {
            KindTab::Mods => return self.choose_view(Page::Discover),
            KindTab::Resource => Shelf::Resource,
            KindTab::Shader => Shelf::Shader,
            KindTab::Data => Shelf::Data,
        };
        self.mods_state.shelf = shelf;
        self.choose_view(Page::PackFind);
    }

    /// The way to what is installed: Installed for mods, Packs for the kind on show.
    fn installed_link(&self, tab: KindTab, cx: &mut Context<Self>) -> Stateful<Div> {
        let page = if tab == KindTab::Mods {
            Page::Installed
        } else {
            Page::Packs
        };
        secondary("alt-discover-installed", tr("Installed")).on_click(cx.listener(
            move |this, _: &ClickEvent, _, cx| {
                this.choose_view(page);
                cx.notify();
            },
        ))
    }

    /// The search field: the find text, Escape to leave it, Enter to search packs.
    fn discover_search(
        &self,
        tab: KindTab,
        focus: &FocusHandle,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = crate::mods::ui::field_theme(self.theme);
        let focused = focus.is_focused(window);
        let packs = tab != KindTab::Mods;
        let placeholder: SharedString = tab.search_placeholder().into();
        let field = crate::widgets::field::search_box(
            focused,
            None,
            line_edit::content_with(
                &self.mods_state.find,
                &t,
                focused,
                false,
                false,
                placeholder,
            ),
        )
        .id("alt-mods-find")
        .track_focus(focus)
        .seen()
        .flex_1()
        .min_w(px(160.))
        .max_w(px(360.))
        .h(px(tokens::ACTION))
        .text_size(px(tokens::BODY))
        .cursor_text()
        .on_click({
            let focus = focus.clone();
            cx.listener(move |_, _: &ClickEvent, window, cx| {
                window.focus(&focus, cx);
                cx.notify();
            })
        })
        .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
            let packs_scope = packs && this.ui.page == Page::PackFind;
            let m = &mut this.mods_state;
            match line_edit::handle(&mut m.find, ev, cx) {
                Key::Escape => window.blur(cx),
                Key::Pass => return,
                Key::Enter if packs_scope => this.pack_search(),
                Key::Edited => {
                    m.filters.text = m.find.text().to_owned();
                    m.limit = 0;
                }
                _ => {}
            }
            cx.stop_propagation();
            cx.notify();
        }));
        line_edit::mouse(field, cx, |this: &mut Launcher| &mut this.mods_state.find)
    }

    /// The profile Mods are added to, as a picker over the profiles.
    fn profile_picker(&self, cx: &mut Context<Self>) -> Div {
        let selected = self.selected_instance().map(|i| i.id.clone());
        let options: Vec<MenuOption> = self
            .instances
            .list
            .iter()
            .map(|instance| {
                let picked = instance.clone();
                (
                    SharedString::from(instance.name.clone()),
                    selected.as_deref() == Some(instance.id.as_str()),
                    Box::new(move |this: &mut Launcher| this.select_instance(&picked))
                        as Box<dyn Fn(&mut Launcher)>,
                )
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .items_end()
            .gap(px(4.))
            .child(muted(tr("Adding to")))
            .child(self.discover_menu(&self.adding_to_name(), Menu::Profile, move || options, cx))
    }

    /// The profile's name, with its build in brackets when it has one.
    pub(in crate::mods) fn adding_to_name(&self) -> String {
        let profile = self
            .selected_instance()
            .map(|i| i.name.clone())
            .unwrap_or_else(|| tr("Default").to_owned());
        match self
            .selected_exact_build()
            .as_deref()
            .map(crate::sheets::build_name)
        {
            Some(build) => trf(
                "{profile} ({build})",
                &[("profile", profile.as_str()), ("build", &build)],
            ),
            None => profile,
        }
    }

    /// Discover's filter row, the same for mods and packs: the category chips on the left; the
    /// Show all switch and the sort menu on the right.
    pub(in crate::mods) fn filter_bar(
        &self,
        categories: Vec<MenuOption>,
        show_all: bool,
        flip_show_all: fn(&mut Launcher),
        sort_menu: Div,
        cx: &mut Context<Self>,
    ) -> Div {
        let show_all_toggle = div()
            .id("alt-show-all")
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .child(switch("alt-show-all-switch", show_all).tab_stop(false))
            .child(div().text_size(px(tokens::LABEL)).child(tr("Show all")))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                flip_show_all(this);
                cx.notify();
            }));
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(self.category_chips(categories, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .child(show_all_toggle)
                    .child(sort_menu),
            )
    }

    /// The first categories as chips (one Tab stop, arrow keys between them), the rest behind a
    /// "more" chip that opens the category menu.
    fn category_chips(&self, mut options: Vec<MenuOption>, cx: &mut Context<Self>) -> Div {
        let overflow = options.split_off(CATEGORY_CHIPS.min(options.len()));
        let active_more = overflow.iter().find(|o| o.1).map(|o| o.0.clone());
        let current = options.iter().position(|o| o.1).unwrap_or(usize::MAX);
        let labels: Vec<(usize, SharedString)> = options
            .iter()
            .enumerate()
            .map(|(n, o)| (n, o.0.clone()))
            .collect();
        let picks: Rc<Vec<Pick>> = Rc::new(options.into_iter().map(|o| o.2).collect());
        let this = cx.entity();
        let chips = crate::widgets::chip::choices(
            "alt-discover-categories",
            &labels,
            current,
            move |n, _, cx| {
                this.update(cx, |view, cx| {
                    picks[n](view);
                    view.mods_state.menu = None;
                    cx.notify();
                });
            },
        );
        let mut row = div()
            .relative()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .child(chips);
        if overflow.is_empty() {
            return row;
        }
        let open = self.mods_state.menu == Some(Menu::Category);
        let more_count = overflow.len().to_string();
        let (more_label, more_kind): (SharedString, _) = match active_more {
            Some(label) => (label, crate::widgets::chip::Kind::Selected),
            None => (
                trf("{count} more", &[("count", more_count.as_str())]).into(),
                crate::widgets::chip::Kind::Neutral,
            ),
        };
        let more = crate::widgets::chip::chip_with(
            "alt-discover-more-categories",
            more_label,
            more_kind,
            None,
            true,
        )
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.mods_state.menu = match this.mods_state.menu {
                Some(Menu::Category) => None,
                _ => Some(Menu::Category),
            };
            cx.notify();
        }));
        row = row.child(more);
        row.children(open.then(|| self.menu_popover(Menu::Category, overflow, cx)))
    }

    /// A drop-down menu: its trigger, and the list of options under it while it is open.
    pub(in crate::mods) fn discover_menu(
        &self,
        label: &str,
        which: Menu,
        options: impl FnOnce() -> Vec<MenuOption>,
        cx: &mut Context<Self>,
    ) -> Div {
        let open = self.mods_state.menu == Some(which);
        let id = match which {
            Menu::Category => "alt-category-menu",
            Menu::Sort => "alt-sort-menu",
            Menu::Profile => "alt-profile-menu",
        };
        div()
            .relative()
            .child(
                select_button(id, label.to_owned(), open)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.mods_state.menu = match this.mods_state.menu {
                            Some(m) if m == which => None,
                            _ => Some(which),
                        };
                        cx.notify();
                    })),
            )
            .children(open.then(|| self.menu_popover(which, options(), cx)))
    }

    /// The list a menu opens under its trigger: the options, the one chosen marked.
    fn menu_popover(
        &self,
        which: Menu,
        items: Vec<MenuOption>,
        cx: &mut Context<Self>,
    ) -> gpui::Deferred {
        let p = palette();
        let left = which == Menu::Category;
        deferred(
            div()
                .absolute()
                .top(px(tokens::CONTROL + 4.))
                .when(left, |d| d.left(px(0.)))
                .when(!left, |d| d.right(px(0.)))
                .min_w(px(190.))
                .max_h(px(300.))
                .id("alt-discover-menu-list")
                .overflow_y_scroll()
                .p(px(4.))
                .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                .bg(c(p.raised))
                .border_1()
                .border_color(c(p.line))
                .shadow_md()
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .flex()
                .flex_col()
                .children(
                    items
                        .into_iter()
                        .enumerate()
                        .map(|(n, (text, active, pick))| {
                            div()
                                .id(("alt-menu-opt", n))
                                .px(px(10.))
                                .py(px(6.))
                                .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                                .text_size(px(tokens::LABEL))
                                .text_color(c(if active { p.accent_ink } else { p.text }))
                                .when(active, |d| d.bg(c(p.selected)))
                                .cursor_pointer()
                                .hover(|s| s.bg(c(p.hover)))
                                .child(text)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    pick(this);
                                    this.mods_state.menu = None;
                                    cx.notify();
                                }))
                        }),
                ),
        )
    }
}

pub(in crate::mods) fn summary_count_line(
    total: usize,
    fitting: usize,
    other: usize,
    show_all: bool,
    profile_name: &str,
    build_label: Option<&str>,
) -> String {
    let target = match build_label {
        Some(build) => trf(
            "{profile} ({build})",
            &[("profile", profile_name), ("build", build)],
        ),
        None => profile_name.to_owned(),
    };
    if show_all && other > 0 {
        trf(
            "{total} mods total · {fit} fit {target} · {other} for other builds",
            &[
                ("total", &total.to_string()),
                ("fit", &fitting.to_string()),
                ("target", &target),
                ("other", &other.to_string()),
            ],
        )
    } else if build_label.is_some() {
        if total == 1 {
            trf("1 mod fits {target}", &[("target", &target)])
        } else {
            trf(
                "{count} mods fit {target}",
                &[("count", &total.to_string()), ("target", &target)],
            )
        }
    } else if total == 1 {
        tr("1 mod").to_owned()
    } else {
        trf("{count} mods", &[("count", &total.to_string())])
    }
}
