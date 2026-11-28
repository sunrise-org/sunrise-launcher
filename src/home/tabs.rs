//! A fixed panel; only the selected collection owns a vertical viewport.
use super::*;
use crate::frame::Page;
use crate::line_edit::LineEdit;
use crate::mods::selection::{Gesture, Selection};
use crate::mods::Installed;
use crate::widgets::check::{checkbox, Tick};
use gpui::{ScrollHandle, StatefulInteractiveElement};

mod cards;
mod logs;
mod scrollbar;
mod search;
mod servers;
mod worlds;

pub(crate) use cards::{Card, ItemAction, MenuEntry, GAP};
pub(crate) use logs::{error_log, Source as LogSource};
pub(crate) use search::{matches as matches_query, no_matches};

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
pub(crate) struct Tabs {
    profile: Option<String>,
    selected: Tab,
    views: [View; 5],
    logs: logs::Logs,
    opener: Option<FocusHandle>,
    /// Stable even as a virtual row is recycled. Cleared when leaving the selected profile.
    rows: HashMap<SharedString, FocusHandle>,
    /// The ticked rows of the Mods tab, the Packs tab and the Worlds tab, in that order.
    picks: [Selection; 3],
    /// The hover of each card, easing in and out, by card id: a card's tick box fades up while
    /// the pointer is on the card.
    fades: HashMap<SharedString, crate::motion::Fade>,
    /// The bar of the list drawn this frame, if rows are ticked: set while the rows are built,
    /// taken when the panel is.
    bar: Option<AnyElement>,
    /// The "…" menu of a mod or pack card, while it is open.
    menu: Option<cards::ItemMenu>,
    /// A switch flip was asked for on a card: its result needs no toast, only a failure does.
    pub(crate) quiet: bool,
    /// What each mod's card says beyond the name, read from Installed once per change.
    pub(crate) facts: crate::mods::selection::item::CardCache,
    /// What each tab's search field holds, by tab: kept while the profile stays selected.
    search: [LineEdit; 5],
    search_focus: [Option<FocusHandle>; 5],
    /// Switches flipped on a card and not yet confirmed by a fresh read of the files: the card
    /// shows the flip at once, and the read replaces it.
    pub(crate) flipped: HashMap<(bool, String), Flip>,
}

/// A flip shown ahead of the files.
pub(crate) struct Flip {
    pub(crate) on: bool,
    /// The inventory revision it was made on: a newer one has the answer.
    pub(crate) revision: u64,
    /// When the change under it ended, so a flip that never reached the files is dropped.
    pub(crate) done: Option<std::time::Instant>,
}

impl Tabs {
    /// The words the Mods tab (or the Packs tab) is searched for, lower-cased; empty for all.
    pub(crate) fn query(&self, packs: bool) -> String {
        self.query_of(if packs { Tab::Packs } else { Tab::Mods })
    }

    fn query_of(&self, tab: Tab) -> String {
        self.search[tab.index()].text().trim().to_lowercase()
    }

    pub(super) fn select_profile(&mut self, id: &str) {
        if self.profile.as_deref() != Some(id) {
            *self = Self {
                profile: Some(id.to_owned()),
                ..Self::default()
            };
        }
    }

    /// The pointer entered (`on`) or left the card `id`: its fade turns around from where it was.
    fn warm(&mut self, id: &SharedString, on: bool) {
        let fade =
            crate::motion::Fade::toward(self.fades.get(id).copied(), on, std::time::Instant::now());
        self.fades.insert(id.clone(), fade);
    }

    /// The fade of the card `id`, while there is something to draw; one that has ended off is
    /// dropped.
    fn lit(&mut self, id: &SharedString) -> Option<crate::motion::Fade> {
        let fade = self.fades.get(id).copied()?;
        if fade.rested(std::time::Instant::now()) {
            self.fades.remove(id);
            return None;
        }
        Some(fade)
    }

    pub(crate) fn remember(&mut self, opener: Option<FocusHandle>) {
        self.opener = opener;
    }

    fn focus_for(&mut self, id: SharedString, cx: &mut Context<Launcher>) -> FocusHandle {
        self.rows
            .entry(id)
            .or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone()
    }
}

/// How a row takes part in ticking: which list it is in, and what the list and its ticks are.
pub(crate) struct Pick {
    /// The Packs tab, else the Mods tab.
    pub(crate) packs: bool,
    pub(crate) key: String,
    /// Every row's key in the order shown, for a Shift range.
    pub(crate) order: std::rc::Rc<Vec<String>>,
    pub(crate) ticked: bool,
    /// Some row of the list is ticked: every row shows its box.
    pub(crate) any: bool,
}

pub(crate) struct Content {
    pub(crate) count: Option<usize>,
    /// While a search is on: how many rows match, of how many.
    pub(crate) shown: Option<(usize, usize)>,
    pub(crate) turned_off: usize,
    pub(crate) manage: Option<Stateful<Div>>,
    pub(crate) items: Div,
}

/// An empty tab: the launcher's empty state, centred in the list's room.
pub(crate) fn empty(title: &'static str, line: &'static str, action: Option<Stateful<Div>>) -> Div {
    crate::widgets::feedback::empty_state(title, line, action.map(|a| a.into_any_element()))
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
            shown,
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
        let searchable = count.is_some_and(|count| count > 0) && active != Tab::Logs;
        let count = count.filter(|&count| count > 0).map(|count| {
            if let Some((shown, total)) = shown {
                trf(
                    "{shown} of {total}",
                    &[("shown", &shown.to_string()), ("total", &total.to_string())],
                )
            } else if turned_off > 0 {
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
        let separator = count.is_some() && off_link.is_some() && shown.is_none();
        let off_link = off_link.filter(|_| shown.is_none());
        let search = searchable.then(|| self.home_search(active, window, cx));
        let bar = self.ui.home.tabs.bar.take();
        let menu = self.home_item_menu(active, cx);
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
                    // 15 px under the tabs and 12 before the first card, around the 32 px field;
                    // the same on every tab that has a search.
                    .pt(px(15.))
                    .pb(px(12.))
                    .h(px(if searchable { 59. } else { 38. }))
                    .items_center()
                    .gap(px(10.))
                    .children(search)
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
            // Under the list, not over it: the last rows stay in reach while rows are ticked.
            .children(bar)
            .children(menu)
    }

    /// Which list the Home tab shown ticks rows in: the Packs tab (true) or the Mods tab
    /// (false). None on any other tab or page.
    pub(crate) fn home_pick_scope(&self) -> Option<bool> {
        if self.ui.page != Page::Home {
            return None;
        }
        match self.ui.home.tabs.selected {
            Tab::Mods => Some(false),
            Tab::Packs => Some(true),
            _ => None,
        }
    }

    /// The ticked rows of the Packs tab (true) or the Mods tab (false).
    pub(crate) fn home_pick(&mut self, packs: bool) -> &mut Selection {
        &mut self.ui.home.tabs.picks[usize::from(packs)]
    }

    /// The ticked rows of the Worlds tab.
    pub(crate) fn home_worlds_pick(&mut self) -> &mut Selection {
        &mut self.ui.home.tabs.picks[2]
    }

    /// Whether Home is showing the Worlds tab, whose rows tick as the Mods tab's do.
    pub(crate) fn home_on_worlds(&self) -> bool {
        self.ui.page == Page::Home && self.ui.home.tabs.selected == Tab::Worlds
    }

    /// Hands the bar of the list being built to the panel that holds it.
    pub(crate) fn home_set_bar(&mut self, bar: Option<AnyElement>) {
        self.ui.home.tabs.bar = bar;
    }

    pub(crate) fn home_installed_list(
        &self,
        packs: bool,
        count: usize,
    ) -> (std::ops::Range<usize>, impl FnOnce(Div) -> Div + use<>) {
        let view = &self.ui.home.tabs.views[if packs { Tab::Packs } else { Tab::Mods }.index()];
        view.near
            .list(&view.scroll, count, 1, WORLD_ROW, WORLD_ROW_GAP)
    }
}
