//! A fixed panel; only the selected collection owns a vertical viewport.
use super::*;
use crate::frame::Page;
use crate::line_edit::LineEdit;
use crate::mods::selection::{Gesture, Selection};
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
    Shaders,
    Servers,
    Logs,
}

/// A list of the profile's files that is ticked, searched and switched card by card: the Mods
/// tab, the Packs tab (resource packs) or the Shaders tab (shader packs).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Collection {
    Mods,
    Resource,
    Shader,
}

impl Collection {
    /// The tab that lists it.
    fn tab(self) -> Tab {
        match self {
            Self::Mods => Tab::Mods,
            Self::Resource => Tab::Packs,
            Self::Shader => Tab::Shaders,
        }
    }

    /// Its place among `Tabs::picks`.
    fn pick_slot(self) -> usize {
        match self {
            Self::Mods => 0,
            Self::Resource => 1,
            Self::Shader => 2,
        }
    }

    /// The kind of pack it lists; None for the mods.
    pub(crate) fn pack_kind(self) -> Option<sunrise_launcher::packs::Kind> {
        match self {
            Self::Mods => None,
            Self::Resource => Some(sunrise_launcher::packs::Kind::Resource),
            Self::Shader => Some(sunrise_launcher::packs::Kind::Shader),
        }
    }

    /// The collection of packs of `kind`.
    pub(crate) fn of_packs(kind: sunrise_launcher::packs::Kind) -> Self {
        match kind {
            sunrise_launcher::packs::Kind::Resource => Self::Resource,
            sunrise_launcher::packs::Kind::Shader => Self::Shader,
        }
    }

    /// Whether it lists packs, not mods.
    pub(crate) fn is_packs(self) -> bool {
        self != Self::Mods
    }
}

/// The place of the Worlds tab's ticks among `Tabs::picks`.
const WORLDS_PICK: usize = 3;

impl Tab {
    /// In the order the profile's tabs stand under its banner.
    const ALL: [Self; 6] = [
        Self::Mods,
        Self::Worlds,
        Self::Packs,
        Self::Shaders,
        Self::Servers,
        Self::Logs,
    ];

    fn index(self) -> usize {
        self as usize
    }

    /// The list it shows, if it is one of the ticked and switched kind.
    fn collection(self) -> Option<Collection> {
        match self {
            Self::Mods => Some(Collection::Mods),
            Self::Packs => Some(Collection::Resource),
            Self::Shaders => Some(Collection::Shader),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Servers => tr("Saved servers"),
            Self::Worlds => tr("Worlds"),
            Self::Packs => tr("Packs"),
            Self::Shaders => tr("Shaders"),
            Self::Mods => tr("Mods"),
            Self::Logs => tr("Logs"),
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
    views: [View; 6],
    logs: logs::Logs,
    opener: Option<FocusHandle>,
    /// Stable even as a virtual row is recycled. Cleared when leaving the selected profile.
    rows: HashMap<SharedString, FocusHandle>,
    /// The ticked rows of the Mods tab, the Packs tab, the Shaders tab and the Worlds tab, in
    /// that order.
    picks: [Selection; 4],
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
    search: [LineEdit; 6],
    search_focus: [Option<FocusHandle>; 6],
    /// Switches flipped on a card and not yet confirmed by a fresh read of the files: the card
    /// shows the flip at once, and the read replaces it.
    pub(crate) flipped: HashMap<(Collection, String), Flip>,
    /// The pack whose details sheet is open (`home_mods::details`).
    pub(crate) pack_details: Option<crate::home_mods::Item>,
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
    /// The words the tab of `collection` is searched for, lower-cased; empty for all.
    pub(crate) fn query(&self, collection: Collection) -> String {
        self.query_of(collection.tab())
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

    /// Whether the pointer is on the card `id` now.
    pub(crate) fn hovered(&self, id: &SharedString) -> bool {
        self.fades.get(id).is_some_and(|fade| fade.on)
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
    /// The list the row is in.
    pub(crate) collection: Collection,
    pub(crate) key: String,
    /// Every row's key in the order shown, for a Shift range.
    pub(crate) order: std::rc::Rc<Vec<String>>,
    pub(crate) ticked: bool,
    /// Some row of the list is ticked: every row shows its box.
    pub(crate) any: bool,
}

pub(crate) struct Content {
    /// How many rows the tab has; with none, the tab shows its empty state and no search.
    pub(crate) count: Option<usize>,
    pub(crate) manage: Option<Stateful<Div>>,
    pub(crate) items: Div,
}

/// An empty tab: the launcher's empty state, centred in the list's room.
pub(crate) fn empty(title: &'static str, line: &'static str, action: Option<Stateful<Div>>) -> Div {
    crate::widgets::feedback::empty_state(title, line, action.map(|a| a.into_any_element()))
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
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let active = self.ui.home.tabs.selected;
        let strip = self.profile_tab_strip(active, cx);
        let Content {
            count,
            manage,
            items,
        } = match active {
            Tab::Logs => return self.home_logs(strip, instance, inset, window, cx),
            Tab::Servers => self.home_servers(instance, cx),
            Tab::Worlds => self.home_worlds(instance, window, cx),
            Tab::Packs | Tab::Shaders | Tab::Mods => {
                let collection = active.collection().unwrap_or(Collection::Mods);
                self.home_installed_tab(collection, cx)
            }
        };
        // An empty tab says so in its body; "0 worlds" over it says nothing more.
        let searchable = count.is_some_and(|count| count > 0) && active != Tab::Logs;
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
                    .children(search),
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

    /// Picks Home's tab for `collection` (Mods, Packs or Shaders), without leaving the page
    /// shown.
    pub(crate) fn select_home_tab(&mut self, collection: Collection) {
        self.ui.home.tabs.selected = collection.tab();
    }

    /// Goes to Home with the tab of `collection` showing: where what used to be Mods ›
    /// Installed and the installed packs and shaders are listed now.
    pub(crate) fn show_home_tab(&mut self, collection: Collection) {
        self.select_home_tab(collection);
        self.navigate(Page::Home);
    }

    /// Which list the Home tab shown ticks rows in. None on any other tab or page.
    pub(crate) fn home_pick_scope(&self) -> Option<Collection> {
        if self.ui.page != Page::Home {
            return None;
        }
        self.ui.home.tabs.selected.collection()
    }

    /// The ticked rows of the tab of `collection`.
    pub(crate) fn home_pick(&mut self, collection: Collection) -> &mut Selection {
        &mut self.ui.home.tabs.picks[collection.pick_slot()]
    }

    /// The ticked rows of the Worlds tab.
    pub(crate) fn home_worlds_pick(&mut self) -> &mut Selection {
        &mut self.ui.home.tabs.picks[WORLDS_PICK]
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
        collection: Collection,
        count: usize,
    ) -> (std::ops::Range<usize>, impl FnOnce(Div) -> Div + use<>) {
        let view = &self.ui.home.tabs.views[collection.tab().index()];
        view.near
            .list(&view.scroll, count, 1, WORLD_ROW, WORLD_ROW_GAP)
    }
}
