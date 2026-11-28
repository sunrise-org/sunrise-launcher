//! The search field in a tab's header row: Mods, Worlds, Packs and Servers each keep their
//! own text while the profile stays selected.
use super::*;
use crate::i18n::tr_key;
use crate::line_edit::{self, Key};
use crate::widgets::field::search_box;

/// True when every word of `query` (already lower-cased) is in one of `fields`.
pub(crate) fn matches(query: &str, fields: &[&str]) -> bool {
    query.is_empty()
        || query.split_whitespace().all(|word| {
            fields
                .iter()
                .any(|field| field.to_lowercase().contains(word))
        })
}

/// What a tab shows when its search finds nothing.
pub(crate) fn no_matches() -> Div {
    empty(tr("No matches"), "", None)
}

impl Launcher {
    /// Whether one of the tabs' search fields has the keyboard.
    pub(crate) fn home_search_focused(&self, window: &Window) -> bool {
        self.ui
            .home
            .tabs
            .search_focus
            .iter()
            .flatten()
            .any(|focus| focus.is_focused(window))
    }

    /// A compact field with a magnifier: Escape clears it (a second Escape leaves it), and a
    /// change of its text clears the ticks and scrolls the list back to its top.
    pub(super) fn home_search(
        &mut self,
        tab: Tab,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let index = tab.index();
        let placeholder = match tab {
            Tab::Mods => tr_key("Search mods"),
            Tab::Worlds => tr_key("Search worlds"),
            Tab::Packs => tr_key("Search packs"),
            Tab::Shaders => tr_key("Search shaders"),
            Tab::Servers => tr_key("Search servers"),
            Tab::Logs => tr_key("Search the log"),
        };
        let focus = self.ui.home.tabs.search_focus[index]
            .get_or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone();
        let t = crate::mods::field_theme(self.theme);
        let focused = focus.is_focused(window);
        let field = search_box(
            focused,
            None,
            line_edit::content(
                &self.ui.home.tabs.search[index],
                &t,
                focused,
                false,
                false,
                placeholder,
            ),
        )
        .id(("home-tab-search", index))
        .track_focus(&focus)
        .seen()
        .flex_none()
        .w(px(220.))
        .h(px(32.))
        .cursor_text()
        .on_click({
            let focus = focus.clone();
            cx.listener(move |_, _: &ClickEvent, window, cx| {
                window.focus(&focus, cx);
                cx.notify();
            })
        })
        .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
            let tabs = &mut this.ui.home.tabs;
            match line_edit::handle(&mut tabs.search[index], ev, cx) {
                Key::Escape => {
                    if tabs.search[index].text().is_empty() {
                        window.blur(cx);
                    } else {
                        tabs.search[index].set(String::new());
                        tabs.searched(index);
                    }
                }
                Key::Pass => return,
                Key::Edited => tabs.searched(index),
                _ => {}
            }
            cx.stop_propagation();
            cx.notify();
        }));
        line_edit::mouse(field, cx, move |this: &mut Launcher| {
            &mut this.ui.home.tabs.search[index]
        })
    }
}

impl Tabs {
    /// The search of tab `index` changed: nothing stays ticked out of sight, and the list starts
    /// from its top.
    fn searched(&mut self, index: usize) {
        let slot = Tab::ALL
            .iter()
            .find(|tab| tab.index() == index)
            .and_then(|tab| tab.collection())
            .map_or(WORLDS_PICK, Collection::pick_slot);
        if let Some(picks) = self.picks.get_mut(slot) {
            picks.clear();
        }
        self.views[index]
            .scroll
            .set_offset(gpui::point(px(0.), px(0.)));
    }
}
