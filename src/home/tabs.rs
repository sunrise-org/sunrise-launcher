//! A fixed panel; only the selected collection owns a vertical viewport.
use super::*;
use crate::frame::Page;
use crate::mods::Installed;
use gpui::{ScrollHandle, StatefulInteractiveElement};

mod logs;
mod scrollbar;
mod servers;
mod worlds;

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub(super) enum Tab {
    Mods,
    #[default]
    Worlds,
    Packs,
    Servers,
    Logs,
}

impl Tab {
    /// In the order the profile's tabs stand under its banner.
    const ALL: [Self; 5] = [
        Self::Mods,
        Self::Worlds,
        Self::Packs,
        Self::Servers,
        Self::Logs,
    ];

    fn index(self) -> usize {
        self as usize
    }

    fn label(self) -> &'static str {
        match self {
            Self::Servers => tr("Saved servers"),
            Self::Worlds => tr("Worlds"),
            Self::Packs => tr("Packs"),
            Self::Mods => tr("Mods"),
            Self::Logs => tr("Logs"),
        }
    }

    fn count(self, count: usize) -> String {
        let (one, many) = match self {
            // The Logs tab has no count line.
            Self::Logs => return String::new(),
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
    views: [View; 5],
    logs: logs::Logs,
    opener: Option<FocusHandle>,
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

pub(crate) struct RowDisplay {
    pub(crate) id: SharedString,
    pub(crate) picture: AnyElement,
    pub(crate) title: String,
    pub(crate) detail: Option<String>,
    pub(crate) badge: Option<crate::mods::ManualBadge>,
    pub(crate) tail: Option<String>,
}

pub(crate) struct Content {
    pub(crate) count: Option<usize>,
    pub(crate) turned_off: usize,
    pub(crate) manage: Option<Stateful<Div>>,
    pub(crate) items: Div,
}

/// An empty tab: the launcher's empty state, centred in the list's room.
pub(crate) fn empty(title: &'static str, line: &'static str, action: Option<Stateful<Div>>) -> Div {
    crate::widgets::feedback::empty_state(title, line, action.map(|a| a.into_any_element()))
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
        .border_color(palette().rule())
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
    /// The profile's tabs: the shared underline tabs, picked by a click or the arrow keys.
    fn profile_tab_strip(&mut self, active: Tab, cx: &mut Context<Self>) -> Stateful<Div> {
        let items: Vec<(Tab, SharedString)> = Tab::ALL
            .iter()
            .map(|tab| (*tab, SharedString::from(tab.label())))
            .collect();
        let this = cx.entity();
        crate::widgets::nav::tabs("home-profile-tabs", &items, active, move |tab, _, cx| {
            this.update(cx, |view, cx| {
                view.ui.home.tabs.selected = tab;
                cx.notify();
            });
        })
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
        let strip = self.profile_tab_strip(active, cx);
        let Content {
            count,
            manage,
            turned_off,
            items,
        } = match active {
            Tab::Logs => return self.home_logs(strip, instance, inset, window, cx),
            Tab::Servers => self.home_servers(instance, compact, cx),
            Tab::Worlds => self.home_worlds(instance, window, cx),
            Tab::Packs | Tab::Mods => self.home_installed_tab(active == Tab::Packs, cx),
        };
        // An empty tab says so in its body; "0 worlds" over it says nothing more.
        let count = count.filter(|&count| count > 0).map(|count| {
            if turned_off > 0 {
                trf("{count} enabled", &[("count", &count.to_string())])
            } else {
                active.count(count)
            }
        });
        let off_link = (turned_off > 0).then(|| {
            div()
                .id("home-off-mods")
                .reach()
                .min_w_0()
                .truncate()
                .cursor_pointer()
                .underline()
                .hover(|s| s.text_color(rgb(palette().text)))
                .child(trf(
                    "{count} turned off",
                    &[("count", &turned_off.to_string())],
                ))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.open_installed(Installed::ModsOff);
                    cx.notify();
                }))
        });
        let separator = count.is_some() && off_link.is_some();
        let view = &self.ui.home.tabs.views[active.index()];
        let scroll = view.scroll.clone();
        crate::frame::scrolls_with(&scroll);
        if let Some(opener) = self.ui.home.tabs.opener.take() {
            let root = self.root_focus.clone();
            let win = window.window_handle();
            cx.defer(move |cx| {
                win.update(cx, |_, window, cx| {
                    if root.contains(&opener, window) {
                        window.focus(&opener, cx);
                    }
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
            // On the page panel itself, under the banner (Figma: no card of its own, the tabs
            // 30 px under the banner's edge).
            .px(px(inset))
            .pt(px(10.))
            // The tabs, with the profile's one action for its collection on the right.
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .gap(px(10.))
                    .child(strip)
                    .children(manage.map(|button| button.flex_none())),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .h(px(38.))
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .items_center()
                            .min_w_0()
                            .gap(px(5.))
                            .text_size(px(tokens::SMALL))
                            .text_color(rgb(palette().muted))
                            .children(count.map(|label| div().min_w_0().truncate().child(label)))
                            .when(separator, |d| d.child(div().flex_none().child("·")))
                            .children(off_link),
                    ),
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
    pub(crate) fn home_overview_row(
        &mut self,
        display: RowDisplay,
        cx: &mut Context<Self>,
        open: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        let RowDisplay {
            id,
            picture,
            title,
            detail,
            badge,
            tail,
        } = display;
        let badge_label = badge.map(|b| b.label().to_owned());
        let full = detail
            .iter()
            .chain(&badge_label)
            .chain(&tail)
            .fold(title.clone(), |label, next| format!("{label} · {next}"));
        let focus = self.ui.home.tabs.focus_for(id.clone(), cx);
        row(id)
            .track_focus(&focus)
            .reach()
            .cursor_pointer()
            .pr(px(12.))
            .tooltip(crate::glass_ui::tip(full))
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
            .children(badge.map(|badge| {
                crate::widgets::chip::tag(badge.label(), crate::widgets::chip::Kind::Neutral)
            }))
            .children(tail.map(|text| muted(text).max_w(px(170.)).truncate()))
            .child(blocks::icon(
                "chevron-right",
                14.,
                rgb(palette().muted).into(),
            ))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.ui.home.tabs.remember(Some(focus.clone()));
                open(this, cx);
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
