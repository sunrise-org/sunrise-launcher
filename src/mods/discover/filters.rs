//! Discover's header and filter row: the title, the kind tabs, the search field, the profile
//! picker, the category chips and the sort menu, and the count line above the list.
use super::*;
use crate::widgets::button::{button, icon_button, Look, Size, Style};
use crate::widgets::dropdown::{Dropdown, Item};
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

impl Launcher {
    /// The page's head and kind tabs; the filter row (search, sort) and the page's own content
    /// follow.
    pub(in crate::mods) fn discover_shell(&mut self, tab: KindTab, cx: &mut Context<Self>) -> Div {
        let header = self.discover_header(tab, cx);
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

    /// The title with Add from source and the profile chip at its end, over the kind tabs with
    /// the way to Installed at theirs. Below 640 px of content, Add from source shrinks to its
    /// icon so the row never wraps.
    fn discover_header(&self, tab: KindTab, cx: &mut Context<Self>) -> Div {
        let narrow = self.content_width() < 640.;
        let add = if narrow {
            icon_button(
                "add-from-source",
                "plus-sign",
                tr("Add from source"),
                Style::Ghost,
            )
        } else {
            button(
                "add-from-source",
                tr("Add from source"),
                Look::new(Style::Ghost, Size::Medium).icon("plus-sign"),
            )
        }
        .seen()
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.open_add_from_source();
            cx.notify();
        }));
        let actions = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(add)
            .child(self.profile_picker(cx))
            .into_any_element();
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(crate::widgets::nav::page_head(
                tr("Discover"),
                Some(actions),
            ))
            .child(crate::widgets::nav::tab_row(
                self.kind_tabs(tab, cx).seen(),
                Some(self.installed_link(tab, cx).into_any_element()),
            ))
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
        button(
            "alt-discover-installed",
            tr("Installed"),
            Look::new(Style::Ghost, Size::Small).icon("package-01"),
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.choose_view(page);
            cx.notify();
        }))
    }

    /// The search field: the find text, Escape to leave it, Enter to search packs.
    pub(in crate::mods) fn discover_search(
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
        .min_w(px(140.))
        .max_w(px(360.))
        .h(px(tokens::FIELD))
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

    /// The profile Mods are added to, as a dropdown over the profiles.
    fn profile_picker(&self, cx: &mut Context<Self>) -> Div {
        let selected = self.selected_instance().map(|i| i.id.clone());
        let adding = |name: &str| trf("Adding to {profile}", &[("profile", name)]);
        let (items, picks): (Vec<Item>, Vec<Pick>) = self
            .instances
            .list
            .iter()
            .map(|instance| {
                let picked = instance.clone();
                let on = selected.as_deref() == Some(instance.id.as_str());
                let shown = adding(&if on {
                    self.adding_to_name()
                } else {
                    instance.name.clone()
                });
                (
                    Item::new(instance.name.clone()).current(on).shown(shown),
                    Box::new(move |this: &mut Launcher| this.select_instance(&picked)) as Pick,
                )
            })
            .unzip();
        self.discover_menu(
            "alt-profile-menu",
            Menu::Profile,
            items,
            picks,
            Some(240.),
            cx,
        )
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

    /// Discover's filter row, the same for mods and packs: the search field and the category
    /// dropdown, then the Show all switch and the sort dropdown.
    pub(in crate::mods) fn filter_bar(
        &self,
        search: Stateful<Div>,
        category_menu: Div,
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
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap(px(12.))
                    .child(search)
                    .child(category_menu),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(14.))
                    .child(show_all_toggle)
                    .child(sort_menu),
            )
    }

    /// A dropdown of Discover's menu `which`: its field, and the list of options under it while
    /// it is open. A pick runs its function on the launcher and closes the menu.
    pub(in crate::mods) fn discover_menu(
        &self,
        id: &'static str,
        which: Menu,
        items: Vec<Item>,
        picks: Vec<Pick>,
        max_width: Option<f32>,
        cx: &mut Context<Self>,
    ) -> Div {
        let open = self.mods_state.menu == Some(which);
        let picks: Rc<Vec<Pick>> = Rc::new(picks);
        let (toggle, close, pick) = (cx.entity(), cx.entity(), cx.entity());
        let mut menu = Dropdown::new(id, items, open)
            .on_toggle(move |_, cx| {
                toggle.update(cx, |view, cx| {
                    let was = view.mods_state.menu;
                    view.close_menus();
                    view.mods_state.menu = match was {
                        Some(m) if m == which => None,
                        _ => Some(which),
                    };
                    cx.notify();
                })
            })
            .on_close(move |_, cx| {
                close.update(cx, |view, cx| {
                    view.mods_state.menu = None;
                    cx.notify();
                })
            })
            .on_pick(move |n, _, cx| {
                pick.update(cx, |view, cx| {
                    picks[n](view);
                    view.mods_state.menu = None;
                    cx.notify();
                })
            });
        if let Some(width) = max_width {
            menu = menu.max_width(width);
        }
        menu.build()
    }
}

/// A menu's options as the dropdown's items and what each does.
pub(in crate::mods) fn menu_parts(options: Vec<MenuOption>) -> (Vec<Item>, Vec<Pick>) {
    options
        .into_iter()
        .map(|(label, current, pick)| (Item::new(label).current(current), pick))
        .unzip()
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
