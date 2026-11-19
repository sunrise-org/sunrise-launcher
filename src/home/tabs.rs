//! A fixed panel; only the selected collection owns a vertical viewport.
use super::*;
use crate::frame::Page;
use gpui::{ScrollHandle, StatefulInteractiveElement};

mod scrollbar;
mod servers;
mod worlds;

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub(super) enum Tab {
    #[default]
    Servers,
    Worlds,
    Packs,
    Mods,
}

impl Tab {
    const ALL: [Self; 4] = [Self::Servers, Self::Worlds, Self::Packs, Self::Mods];

    fn index(self) -> usize {
        self as usize
    }

    fn label(self) -> &'static str {
        match self {
            Self::Servers => tr("Servers"),
            Self::Worlds => tr("Worlds"),
            Self::Packs => tr("Resource Packs"),
            Self::Mods => tr("Mods"),
        }
    }

    fn count(self, count: usize) -> String {
        let (one, many) = match self {
            Self::Servers => ("1 saved server", "{count} saved servers"),
            Self::Worlds => ("1 world", "{count} worlds"),
            Self::Packs => ("1 resource pack", "{count} resource packs"),
            Self::Mods => ("1 enabled mod", "{count} enabled mods"),
        };
        if count == 1 {
            tr(one).into()
        } else {
            trf(many, &[("count", &count.to_string())])
        }
    }
}

#[derive(Default)]
struct View {
    scroll: ScrollHandle,
    near: Near,
    scrollbar: scrollbar::Thumb,
}

#[derive(Default)]
pub(super) struct Tabs {
    profile: Option<String>,
    selected: Tab,
    views: [View; 4],
    strip_scroll: ScrollHandle,
    strip_focus: Option<FocusHandle>,
    opener: Option<FocusHandle>,
    server: Option<servers::Target>,
    /// Stable even as a virtual row is recycled. Cleared when leaving the selected profile.
    rows: HashMap<SharedString, FocusHandle>,
}

impl Tabs {
    pub(super) fn select_profile(&mut self, id: &str) {
        if self.profile.as_deref() != Some(id) {
            *self = Self {
                profile: Some(id.to_owned()),
                ..Self::default()
            };
        }
    }

    pub(super) fn remember(&mut self, opener: Option<FocusHandle>) {
        self.opener = opener;
    }

    fn focus_for(&mut self, id: SharedString, cx: &mut Context<Launcher>) -> FocusHandle {
        self.rows
            .entry(id)
            .or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone()
    }
}

pub(crate) struct Content {
    pub(crate) count: Option<usize>,
    pub(crate) extra: Option<Stateful<Div>>,
    pub(crate) manage: Option<Stateful<Div>>,
    pub(crate) items: Div,
}

pub(crate) fn empty(message: &'static str, action: Stateful<Div>) -> Div {
    div()
        .flex_none()
        .min_h(gpui::relative(1.))
        .py(px(8.))
        .flex()
        .items_center()
        .justify_center()
        .flex_wrap()
        .gap(px(8.))
        .child(muted(message))
        .child(action)
}

pub(crate) fn row(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex_none()
        .h(px(ROW))
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(6.))
        .border_b_1()
        .border_color(rgb(palette().divider))
        .hover(|s| s.bg(rgb(palette().chrome_raised)))
}

pub(crate) fn row_name(name: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .text_size(px(tokens::LABEL))
        .font_weight(FontWeight::SEMIBOLD)
        .truncate()
        .child(name.into())
}

impl Launcher {
    fn profile_tab_strip(
        &mut self,
        active: Tab,
        focus: &FocusHandle,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let scroll = self.ui.home.tabs.strip_scroll.clone();
        let mut strip = div()
            .id("home-profile-tabs")
            .flex()
            .flex_none()
            .h(px(44.))
            .min_w_0()
            .overflow_x_scroll()
            .track_scroll(&scroll)
            .border_b_1()
            .border_color(rgb(palette().divider));
        for tab in Tab::ALL {
            let selected = tab == active;
            strip = strip.child(
                div()
                    .id(("home-profile-tab", tab.index()))
                    .relative()
                    .flex()
                    .flex_none()
                    .h_full()
                    .items_center()
                    .px(px(if compact { 9. } else { 12. }))
                    .text_size(px(14.))
                    .cursor_pointer()
                    .whitespace_nowrap()
                    .text_color(rgb(if selected {
                        palette().text
                    } else {
                        palette().muted
                    }))
                    .hover(|s| s.bg(rgb(palette().chrome_raised)))
                    .when(selected, |d| {
                        d.track_focus(focus)
                            .reach()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(
                                div()
                                    .absolute()
                                    .bottom_0()
                                    .left_0()
                                    .right_0()
                                    .h(px(2.))
                                    .bg(rgb(palette().accent)),
                            )
                    })
                    .child(tab.label())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.ui.home.tabs.selected = tab;
                        // Pointer and keyboard both land on the one roving tab stop.
                        if let Some(focus) = &this.ui.home.tabs.strip_focus {
                            window.focus(focus, cx);
                        }
                        cx.notify();
                    }))
                    .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
                        if !selected || ev.keystroke.modifiers.modified() {
                            return;
                        }
                        let next = match ev.keystroke.key.as_str() {
                            "left" => (tab.index() + Tab::ALL.len() - 1) % Tab::ALL.len(),
                            "right" => (tab.index() + 1) % Tab::ALL.len(),
                            "home" => 0,
                            "end" => Tab::ALL.len() - 1,
                            _ => return,
                        };
                        this.ui.home.tabs.selected = Tab::ALL[next];
                        this.ui.home.tabs.strip_scroll.scroll_to_item(next);
                        cx.stop_propagation();
                        cx.notify();
                    })),
            );
        }
        strip
    }

    pub(super) fn profile_tabs(
        &mut self,
        instance: &instances::Instance,
        inset: f32,
        compact: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let active = self.ui.home.tabs.selected;
        let focus = self
            .ui
            .home
            .tabs
            .strip_focus
            .get_or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone();
        let strip = self.profile_tab_strip(active, &focus, compact, cx);
        let Content {
            count,
            manage,
            extra,
            items,
        } = match active {
            Tab::Servers => self.home_servers(instance, compact, window, cx),
            Tab::Worlds => self.home_worlds(instance, compact, window, cx),
            Tab::Packs | Tab::Mods => self.home_installed_tab(active == Tab::Packs, window, cx),
        };
        let view = &self.ui.home.tabs.views[active.index()];
        let scroll = view.scroll.clone();
        crate::frame::scrolls_with(&scroll);
        if let Some(opener) = self.ui.home.tabs.opener.take() {
            let root = self.root_focus.clone();
            let win = window.window_handle();
            cx.defer(move |cx| {
                win.update(cx, |_, window, cx| {
                    window.focus(
                        if root.contains(&opener, window) {
                            &opener
                        } else {
                            &focus
                        },
                        cx,
                    );
                })
                .ok();
            });
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(crate::tokens::r(crate::tokens::R_PANEL))
            .bg(palette().chrome_plate())
            .border_1()
            .border_color(rgb(palette().divider))
            .px(px(inset - 1.))
            .pb(px(8.))
            .child(strip)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .h(px(38.))
                    .items_center()
                    .justify_between()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .min_w_0()
                            .gap(px(10.))
                            .children(count.map(|count| muted(active.count(count)).truncate()))
                            .children(extra),
                    )
                    .children(manage),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id(("home-profile-list", active.index()))
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .overflow_y_scroll()
                            .track_scroll(&scroll)
                            .on_scroll(cx.listener(|_, _, _, cx| cx.notify()))
                            .on_key_down(cx.listener(move |_, ev: &KeyDownEvent, _, cx| {
                                let direction = match ev.keystroke.key.as_str() {
                                    "pageup" => 1.,
                                    "pagedown" => -1.,
                                    _ => return,
                                };
                                let page = (scroll.bounds().size.height - px(ROW)).max(px(ROW));
                                let old = scroll.offset();
                                scroll.set_offset(gpui::point(
                                    old.x,
                                    (old.y + page * direction)
                                        .clamp(-scroll.max_offset().y, px(0.)),
                                ));
                                cx.stop_propagation();
                                cx.notify();
                            }))
                            .child(items),
                    )
                    .child(view.scrollbar.draw(&view.scroll, cx)),
            )
    }

    /// The shared read-only Home row. Destination pages own all mutations.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn home_overview_row(
        &mut self,
        id: SharedString,
        picture: AnyElement,
        title: String,
        detail: Option<String>,
        tail: Option<String>,
        window: &Window,
        cx: &mut Context<Self>,
        open: impl Fn(&mut Self) + 'static,
    ) -> Stateful<Div> {
        let full = detail
            .iter()
            .chain(&tail)
            .fold(title.clone(), |label, next| format!("{label} · {next}"));
        let focus = self.ui.home.tabs.focus_for(id.clone(), cx);
        let keyboard_title = (focus.is_focused(window)
            && window.last_input_was_keyboard()
            && title.chars().count() > 50)
            .then(|| {
                gpui::deferred(
                    div().absolute().left(px(42.)).bottom_full().child(
                        gpui::anchored().snap_to_window_with_margin(px(8.)).child(
                            div()
                                .max_w(px(380.))
                                .px(px(10.))
                                .py(px(7.))
                                .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                                .border_1()
                                .border_color(rgb(palette().divider))
                                .bg(rgb(palette().raised))
                                .text_size(px(tokens::LABEL))
                                .child(full.clone()),
                        ),
                    ),
                )
            });
        row(id)
            .track_focus(&focus)
            .aria_label(full.clone())
            .reach()
            .cursor_pointer()
            .pr(px(12.))
            .tooltip(crate::glass_ui::tip(full))
            .children(keyboard_title)
            .child(picture)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(row_name(title))
                    .children(
                        detail.map(|text| muted(text).flex_none().truncate().max_w(px(130.))),
                    ),
            )
            .children(tail.map(|text| muted(text).max_w(px(170.)).truncate()))
            .child(blocks::icon(
                "chevron-right",
                14.,
                rgb(palette().muted).into(),
            ))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.ui.home.tabs.remember(Some(focus.clone()));
                open(this);
                cx.notify();
            }))
    }

    pub(crate) fn home_installed_list(
        &self,
        packs: bool,
        count: usize,
    ) -> (std::ops::Range<usize>, impl FnOnce(Div) -> Div + use<>) {
        let view = &self.ui.home.tabs.views[if packs { Tab::Packs } else { Tab::Mods }.index()];
        view.near.list(&view.scroll, count, 1, ROW, 0.)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_profile_starts_at_servers_with_fresh_scroll_handles() {
        let mut tabs = Tabs::default();
        tabs.select_profile("juniper");
        tabs.selected = Tab::Packs;
        let saved = &tabs.views[Tab::Packs.index()].scroll;
        saved.set_offset(gpui::point(px(0.), px(-480.)));
        tabs.select_profile("juniper");
        assert!(tabs.selected == Tab::Packs);
        assert_eq!(tabs.views[Tab::Packs.index()].scroll.offset().y, px(-480.));
        tabs.select_profile("birch");
        assert!(tabs.selected == Tab::Servers);
        assert_eq!(tabs.views[Tab::Packs.index()].scroll.offset().y, px(0.));
    }
}
