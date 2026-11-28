//! Search (Ctrl+K): one field that finds a place in the launcher and goes there.
//!
//! It lists what the player has, never a catalogue: the launcher's pages and the sections of
//! Settings, the profiles, the selected profile's worlds, its installed mods and packs, and the
//! rows of Settings by name. Results are grouped by kind under a heading, the group with the
//! best match first, each row with its own picture: a mod's icon, a profile's mark, a world's
//! own picture, a pack's, and the pages' own icons for the rest.
//!
//! It is a sheet (`Sheet::Search`), so the frame's rule for layers holds: it takes the keyboard
//! when it opens and gives it back to whatever had it. Typing goes to the field from anywhere
//! in it; the arrows move through the rows across groups, Enter opens one, Escape closes.
//!
//! It grew from 1.0.x's palette (`glass_palette.rs`): bounded results, the same keys. It no
//! longer starts the game, installs or removes anything: it only goes to places.
use super::*;
use crate::frame::{Page, Reach as _};
use crate::line_edit::{self, Key, LineEdit};
use crate::paint::palette;
use crate::settings::Section;
use crate::sheets::Sheet;
use crate::widgets::field::search_box;
use gpui::{rgb, MouseButton, MouseDownEvent};
use std::path::PathBuf;
use std::sync::Arc;
use sunrise_launcher::packs::Kind as PackKind;

mod listing;
use listing::{destinations, keys, results, shortcuts, Art, Entry, Group, Target};
#[cfg(test)]
use listing::{rank, PER_GROUP, SETTINGS_ROWS, SHOWN};

/// The side of a row's picture.
const ART: f32 = 28.;
const WIDTH: f32 = 620.;
/// A result row: one line, its picture, label and hint.
const ROW_H: f32 = 36.;
/// From this width the title bar's field shows its key and the long placeholder.
const SEARCH_ROOMY: f32 = 260.;

/// The title bar's search field width for a `window` this wide: what the middle of the bar
/// leaves between the brand (about 200) and the account and window controls (about 260) on
/// each side, between 160 and 300.
pub(crate) fn search_width(window: f32) -> f32 {
    (window - 2. * 260.).clamp(160., 300.)
}

/// What a key did.
#[derive(PartialEq)]
pub(crate) enum Press {
    /// The search took it and stays open.
    Stay,
    Open(Target),
    Close,
    /// Not the search's: a shortcut.
    Pass,
}

#[derive(Default)]
pub(crate) struct Search {
    pub(crate) query: LineEdit,
    entries: Vec<Entry>,
    /// The rows listed, as indexes into `entries`, and the one the keyboard is on.
    shown: Vec<usize>,
    index: usize,
    /// The worlds the entries were gathered with: the folder they are of, and how many.
    worlds: (Option<PathBuf>, usize),
    scroll: gpui::ScrollHandle,
}

impl Search {
    fn new(entries: Vec<Entry>, worlds: (Option<PathBuf>, usize)) -> Search {
        let mut search = Search {
            query: LineEdit::new(String::new(), 80, 48),
            entries,
            worlds,
            ..Search::default()
        };
        search.list();
        search
    }

    /// Lists the rows for the query as it is now, the keyboard on the first.
    fn list(&mut self) {
        self.shown = results(&self.entries, self.query.text());
        self.index = 0;
    }

    fn rows(&self) -> impl Iterator<Item = &Entry> {
        self.shown.iter().map(|&i| &self.entries[i])
    }

    /// A key pressed anywhere in the search. The arrows move through the rows, round the ends;
    /// Tab stays; the rest is the field's (`field` hands it the key), whose Enter opens the row
    /// the keyboard is on and whose Escape closes.
    pub(crate) fn press(&mut self, key: &str, field: impl FnOnce(&mut LineEdit) -> Key) -> Press {
        let rows = self.shown.len();
        match key {
            "down" if rows > 0 => self.index = (self.index + 1) % rows,
            "up" if rows > 0 => self.index = (self.index + rows - 1) % rows,
            "down" | "up" | "tab" => {}
            _ => match field(&mut self.query) {
                Key::Enter => {
                    if let Some(entry) = self.rows().nth(self.index) {
                        return Press::Open(entry.target.clone());
                    }
                }
                Key::Escape => return Press::Close,
                Key::Edited => self.list(),
                Key::Pass => return Press::Pass,
                _ => {}
            },
        }
        Press::Stay
    }
}

impl Launcher {
    /// Everything the search can find, gathered when it opens.
    fn search_entries(&mut self) -> Vec<Entry> {
        let mut all = destinations();
        all.extend(shortcuts());
        all.extend(self.instances.list.iter().map(|profile| Entry {
            group: Group::Profiles,
            label: profile.name.clone(),
            hint: tr("Profile").to_string(),
            target: Target::Profile(profile.id.clone()),
            art: Art::Profile(profile.id.clone()),
            quiet: false,
        }));
        all.extend(self.worlds.list.iter().map(|world| {
            Entry {
                group: Group::Worlds,
                label: world.name.clone(),
                hint: tr(world.mode).to_string(),
                target: Target::World,
                art: Art::World(
                    world.icon.clone(),
                    world
                        .last_played
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs()),
                ),
                quiet: false,
            }
        }));
        all.extend(
            self.mods_for_search()
                .into_iter()
                .map(|(id, title, icon)| Entry {
                    group: Group::Mods,
                    label: title.clone(),
                    hint: tr("Mod").to_string(),
                    target: Target::Mod(title),
                    art: Art::Mod(id, icon),
                    quiet: false,
                }),
        );
        all.extend(
            self.packs_for_search()
                .into_iter()
                .map(|(kind, file, title, icon)| Entry {
                    group: Group::Packs,
                    label: title.clone(),
                    hint: tr(match kind {
                        PackKind::Resource => "Resource pack",
                        PackKind::Shader => "Shader pack",
                    })
                    .to_string(),
                    target: Target::Pack(kind, title),
                    art: Art::Pack(file, icon),
                    quiet: false,
                }),
        );
        all
    }

    fn search_worlds(&self) -> (Option<PathBuf>, usize) {
        (self.worlds.home.clone(), self.worlds.list.len())
    }

    /// Ctrl+K: opens the search, or closes it when it is open. It stays shut, and says so by
    /// returning false, while anything else is over the page.
    pub(crate) fn toggle_search(&mut self) -> bool {
        if self.ui.sheet == Some(Sheet::Search) && !self.blocking_dialog() {
            self.close_sheet();
            return true;
        }
        if self.layer_open() {
            return false;
        }
        let entries = self.search_entries();
        self.search = Search::new(entries, self.search_worlds());
        self.open_sheet(Sheet::Search);
        true
    }

    fn search_go(&mut self, target: Target, cx: &mut Context<Self>) {
        if target == Target::Search {
            return;
        }
        self.close_sheet();
        match target {
            Target::Page(page) => self.navigate(page),
            Target::Section(section) => {
                self.settings_open(section);
                self.navigate(Page::Settings);
            }
            Target::WorldsOnHome => {
                self.settings_open(Section::Appearance);
                self.ui.settings.appearance.ask_for_worlds();
                self.navigate(Page::Settings);
            }
            Target::Profile(id) => {
                if let Some(profile) = self.instances.list.iter().find(|i| i.id == id).cloned() {
                    self.switch_profile(&profile, cx);
                }
            }
            Target::World => self.navigate(Page::Home),
            Target::Mod(title) => {
                if !self.open_mod_named(&title) {
                    self.show_home_tab(crate::home::tabs::Collection::Mods);
                }
            }
            Target::Pack(kind, title) => self.open_pack_named(kind, title),
            Target::Search => {}
            Target::Sidebar => self.toggle_sidebar(),
        }
    }

    /// The top bar's way in: a field to press. Its key is shown whole where the bar has room
    /// for it and not at all where it has not; on a narrow window the field is its icon alone,
    /// with the word as its tooltip.
    pub(crate) fn search_button(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        // The field sits in the middle of the title bar, between the brand and the account: it
        // is always a field, narrowing with the window (300 wide where it fits, never under
        // 160), and drops its key and long words only when it is narrow.
        let width = search_width(self.window_width);
        let roomy = width >= SEARCH_ROOMY;
        let key = roomy.then(|| SharedString::from(keys("K")));
        let word = div()
            .text_size(px(tokens::LABEL))
            .text_color(rgb(palette().chrome_muted))
            .truncate()
            .child(if roomy {
                tr("Search profiles, mods and packs")
            } else {
                tr("Search…")
            });
        let body = search_box(false, key, word).h(px(34.)).w(px(width));
        let button = body
            .id("search-open")
            .flex_none()
            .hover(|s| s.bg(rgb(palette().chrome_raised)))
            .cursor_pointer()
            .reach_edge()
            .on_click(cx.listener(|this, _, _, cx| {
                this.toggle_search();
                cx.notify();
            }));
        button
    }

    /// A row's picture, `ART` square.
    fn search_art(&mut self, art: &Art, label: &str) -> Div {
        let tile = || {
            div()
                .flex_none()
                .size(px(ART))
                .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                .overflow_hidden()
                .flex()
                .items_center()
                .justify_center()
        };
        let glyph = |name: &'static str| {
            tile().bg(rgb(palette().field)).child(blocks::icon(
                name,
                16.,
                rgb(palette().muted).into(),
            ))
        };
        match art {
            Art::Glyph(name) => glyph(name),
            Art::Profile(id) => match self.instances.list.iter().find(|i| &i.id == id).cloned() {
                Some(profile) => self.profile_mark(&profile, ART),
                None => glyph("box"),
            },
            Art::World(icon, stamp) => {
                let picture = icon.clone().and_then(|file| {
                    let key = format!("search-world:{}:{stamp}", file.display());
                    self.icon_image(key, ART, move || Some(file))
                });
                match picture {
                    Some(image) => tile().child(gpui::img(image).size(px(ART))),
                    None => glyph("map"),
                }
            }
            Art::Mod(id, icon) => {
                let picture = icon
                    .as_ref()
                    .and_then(|url| self.mods_image(url, sunrise_launcher::browser::images::ROW));
                crate::mod_icons::tile(picture, id, label, ART)
            }
            Art::Pack(file, image) => crate::mod_icons::tile(image.clone(), file, label, ART),
        }
    }

    /// The search, as the layer the frame draws over the page.
    pub(crate) fn command_search(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        // The worlds are read in the background: they join the list when they arrive.
        if let Some(home) = self.selected_instance().map(|i| i.home.clone()) {
            self.load_worlds(home, cx);
        }
        if self.search.worlds != self.search_worlds() {
            self.search.entries = self.search_entries();
            self.search.worlds = self.search_worlds();
            let index = self.search.index;
            self.search.list();
            self.search.index = index.min(self.search.shown.len().saturating_sub(1));
        }
        let rows: Vec<Entry> = self.search.rows().cloned().collect();
        let index = self.search.index;
        let query = self.search.query.text().trim().to_string();

        let field = search_box(
            true,
            Some(SharedString::from(tr("Esc"))),
            line_edit::content(
                &self.search.query,
                &self.theme,
                true,
                false,
                false,
                "Search pages, profiles, worlds, mods, settings and shortcuts…",
            ),
        )
        .id("search-field")
        .flex_none()
        .gap(px(12.))
        .h(px(tokens::FIELD))
        .px(px(12.))
        .cursor_text()
        .text_size(px(tokens::BODY));
        let field = line_edit::mouse(field, cx, |this: &mut Launcher| &mut this.search.query);

        let mut list = div()
            .id("search-results")
            .flex()
            .flex_col()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.search.scroll)
            .px(px(8.))
            .pb(px(8.));
        let mut group = None;
        for (n, entry) in rows.into_iter().enumerate() {
            if group != Some(entry.group) {
                group = Some(entry.group);
                list = list.child(
                    div()
                        .flex_none()
                        .px(px(8.))
                        .pt(px(10.))
                        .pb(px(4.))
                        .text_size(px(tokens::SMALL))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette().muted))
                        .child(tr(entry.group.heading())),
                );
            }
            let target = entry.target.clone();
            list = list.child(
                div()
                    .id(ElementId::named_usize("search-row", n))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .h(px(ROW_H))
                    .px(px(10.))
                    .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                    .cursor_pointer()
                    .when(n == index, |d| d.bg(rgb(palette().selected)))
                    .child(self.search_art(&entry.art, &entry.label))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(entry.label),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(tokens::SMALL))
                            .text_color(rgb(palette().muted))
                            // A shortcut's key reads as a key.
                            .when(entry.group == Group::Shortcuts, |d| {
                                d.px(px(6.))
                                    .py(px(2.))
                                    .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                                    .bg(rgb(palette().field))
                                    .font_weight(FontWeight::SEMIBOLD)
                            })
                            .child(entry.hint),
                    )
                    .on_mouse_move(cx.listener(move |this, _, _, cx| {
                        if this.search.index != n {
                            this.search.index = n;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.search_go(target.clone(), cx);
                        cx.notify();
                    })),
            );
        }
        if self.search.shown.is_empty() {
            list = list.child(
                div()
                    .px(px(8.))
                    .py(px(18.))
                    .text_color(rgb(palette().muted))
                    .child(trf("Nothing matches “{query}”.", &[("query", &query)])),
            );
        }

        let top = (self.window_height * 0.12).max(16.);
        let card = div()
            .w(px(WIDTH))
            .max_w_full()
            .max_h(px((self.window_height - top - 32.).max(0.)))
            .flex()
            .flex_col()
            .rounded(crate::tokens::r(crate::tokens::R_PANEL))
            // The same faint edge as the menus and sheets, not the strong outline.
            .border_1()
            .border_color(palette().input_edge())
            .bg(rgb(palette().raised))
            .text_color(rgb(palette().text))
            .shadow_lg()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation()
            })
            .child(div().flex_none().p(px(8.)).child(field))
            .child(list)
            .child(
                div()
                    .flex_none()
                    .px(px(16.))
                    .pt(px(4.))
                    .pb(px(10.))
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(palette().muted))
                    .child(tr("↑ ↓ to move · Enter to open · Esc to close")),
            );
        div()
            .id("search")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .justify_center()
            .items_start()
            .pt(px(top))
            .px(px(16.))
            .bg(palette().scrim())
            .on_scroll(|_, _, cx| cx.stop_propagation())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.close_sheet();
                    cx.notify();
                }),
            )
            // On the layer, which has the keyboard: typing reaches the field from anywhere.
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                let pressed = this.search.press(ev.keystroke.key.as_str(), |field| {
                    line_edit::handle(field, ev, cx)
                });
                match pressed {
                    Press::Pass => return,
                    Press::Open(target) => this.search_go(target, cx),
                    Press::Close => this.close_sheet(),
                    Press::Stay => {
                        // The row the keyboard is on shows; a heading above each group is a
                        // child of the list too.
                        let s = &this.search;
                        let headings = s
                            .rows()
                            .take(s.index + 1)
                            .fold((0, None), |(count, last), entry| {
                                if last == Some(entry.group) {
                                    (count, last)
                                } else {
                                    (count + 1, Some(entry.group))
                                }
                            })
                            .0;
                        s.scroll.scroll_to_item(s.index + headings);
                    }
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .child(card)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(group: Group, label: &str) -> Entry {
        Entry {
            group,
            label: label.into(),
            hint: String::new(),
            target: Target::World,
            art: Art::Glyph("box"),
            quiet: false,
        }
    }

    /// The launcher's own places and a player's things beside them.
    fn sample() -> Vec<Entry> {
        let mut all = destinations();
        all.extend(shortcuts());
        all.extend([
            entry(Group::Profiles, "Sunrise"),
            entry(Group::Profiles, "Wildflower Valley"),
            entry(Group::Worlds, "Birch Run"),
            entry(Group::Worlds, "Meadow Pond"),
            entry(Group::Mods, "JEI"),
            entry(Group::Mods, "Mod Menu"),
            entry(Group::Packs, "Meadow Shaders"),
        ]);
        all
    }

    fn listed(entries: &[Entry], query: &str) -> Vec<(Group, String)> {
        results(entries, query)
            .into_iter()
            .map(|i| (entries[i].group, entries[i].label.clone()))
            .collect()
    }

    fn labels(entries: &[Entry], query: &str) -> Vec<String> {
        listed(entries, query).into_iter().map(|(_, l)| l).collect()
    }

    #[test]
    fn a_word_ranks_by_where_it_is_found() {
        assert_eq!(rank("home", "Home"), Some(0));
        assert_eq!(rank("ho", "Home"), Some(1));
        assert_eq!(rank("host", "Servers › Host"), Some(2));
        assert_eq!(rank("ost", "Servers › Host"), Some(3));
        assert_eq!(rank("zz", "Home"), None);
        // Every word must be found; each adds its own place.
        assert_eq!(rank("servers ho", "Servers › Host"), Some(1 + 2));
        assert_eq!(rank("servers zz", "Servers › Host"), None);
        assert_eq!(rank("WILD", "Wildflower Valley"), Some(1));
    }

    #[test]
    fn nothing_typed_lists_the_pages_then_the_profiles() {
        let entries = sample();
        assert_eq!(
            labels(&entries, "  "),
            [
                "Home",
                "Mods",
                "Packs",
                "Servers › Join",
                "Servers › Host",
                "Skins",
                "Settings",
                "Sunrise",
                "Wildflower Valley",
                "Search",
                "Show or hide the sidebar",
            ]
        );
    }

    #[test]
    fn packs_search_opens_the_packs_destination() {
        let mut search = Search::new(sample(), (None, 0));
        search.query.set("Packs".to_string());
        search.list();

        let first = search.rows().next().unwrap();
        assert_eq!(first.group, Group::Navigate);
        assert_eq!(first.label, "Packs");
        assert!(matches!(
            search.press("enter", |_| Key::Enter),
            Press::Open(Target::Page(Page::Packs))
        ));
    }

    #[test]
    fn the_shortcuts_are_listed_with_their_keys_however_many_profiles_there_are() {
        let mut entries = sample();
        entries.extend((0..40).map(|n| entry(Group::Profiles, &format!("Profile {n}"))));
        let found = results(&entries, "");
        assert_eq!(found.len(), SHOWN);
        let shown: Vec<&str> = found[SHOWN - 2..]
            .iter()
            .map(|&i| entries[i].hint.as_str())
            .collect();
        assert_eq!(shown, [keys("K"), keys("B")]);
        // Found by name too, and each does what its key does.
        let i = results(&entries, "sidebar")[0];
        assert!(entries[i].target == Target::Sidebar);
    }

    #[test]
    fn results_are_grouped_and_the_group_with_the_best_match_comes_first() {
        let entries = sample();
        // "Meadow Pond" and "Meadow Shaders" start with it: worlds were gathered first.
        assert_eq!(
            listed(&entries, "mea"),
            [
                (Group::Worlds, "Meadow Pond".to_string()),
                (Group::Packs, "Meadow Shaders".to_string()),
            ]
        );
        // The mod called exactly that leads, ahead of the pages that start with the word.
        let found = listed(&entries, "mod menu");
        assert_eq!(found[0], (Group::Mods, "Mod Menu".to_string()));
        // "Mods" is a whole label: its group leads.
        let found = listed(&entries, "mods");
        assert_eq!(found[0], (Group::Navigate, "Mods".to_string()));
        let groups: Vec<Group> = found.iter().map(|(g, _)| *g).collect();
        let mut seen = groups.clone();
        seen.dedup();
        let mut once = seen.clone();
        once.sort_by_key(|g| *g as u8);
        once.dedup();
        assert_eq!(
            seen.len(),
            once.len(),
            "each group is listed once: {groups:?}"
        );
        // A row of Settings is found by its own title, in its own group.
        assert!(listed(&entries, "arguments")
            .contains(&(Group::Settings, "Game arguments".to_string())));
    }

    #[test]
    fn settings_sections_and_rows_are_found_only_by_name() {
        let entries = sample();
        assert!(!labels(&entries, "").iter().any(|l| l.contains("Privacy")));
        assert_eq!(labels(&entries, "priv"), ["Settings › Privacy"]);
        let found = listed(&entries, "language");
        assert_eq!(found, [(Group::Settings, "Language".to_string())]);
        let i = results(&entries, "language")[0];
        assert!(entries[i].target == Target::Section(Section::Launcher));
        assert_eq!(entries[i].hint, "Settings › Launcher");
    }

    #[test]
    fn build_management_search_goes_to_storage() {
        let entries = sample();
        for query in [
            "installed game builds",
            "import a game build",
            "remove old game builds",
        ] {
            let entry = &entries[results(&entries, query)[0]];
            assert_eq!(entry.group, Group::Settings);
            assert_eq!(entry.hint, "Settings › Storage");
            assert!(entry.target == Target::Section(Section::Storage));
        }
    }

    /// The rows named here must be the ones Settings draws.
    #[test]
    fn the_settings_rows_are_the_ones_settings_has() {
        let source = concat!(
            include_str!("settings.rs"),
            include_str!("settings/account_section.rs"),
            include_str!("settings/appearance.rs"),
            include_str!("settings/appearance/rows.rs"),
            include_str!("settings/appearance/colours.rs"),
            include_str!("settings/appearance/theme_library.rs"),
            include_str!("settings/game.rs"),
            include_str!("settings/launcher.rs"),
            include_str!("settings/privacy.rs"),
            include_str!("settings/storage_section.rs"),
            include_str!("storage_builds.rs")
        );
        for (title, _) in SETTINGS_ROWS {
            assert!(source.contains(&format!("tr(\"{title}\")")), "{title}");
        }
    }

    #[test]
    fn results_are_bounded() {
        let mut entries = sample();
        entries.extend((0..40).map(|n| entry(Group::Mods, &format!("Meadow mod {n}"))));
        entries.extend((0..40).map(|n| entry(Group::Worlds, &format!("Meadow world {n}"))));
        entries.extend((0..40).map(|n| entry(Group::Profiles, &format!("Meadow {n}"))));
        let found = listed(&entries, "meadow");
        assert!(found.len() <= SHOWN);
        for group in [Group::Mods, Group::Worlds, Group::Profiles, Group::Packs] {
            let of = found.iter().filter(|(g, _)| *g == group).count();
            assert!((1..=PER_GROUP).contains(&of), "{group:?}: {of}");
        }
        assert!(results(&entries, "").len() <= SHOWN);
    }

    #[test]
    fn nothing_retired_is_listed() {
        let places: Vec<String> = destinations().into_iter().map(|e| e.label).collect();
        assert_eq!(
            places[..7],
            [
                "Home",
                "Mods",
                "Packs",
                "Servers › Join",
                "Servers › Host",
                "Skins",
                "Settings"
            ]
        );
        assert_eq!(places.len(), 7 + Section::ALL.len() + SETTINGS_ROWS.len());
        let entries = sample();
        for gone in [
            "java",
            "modrinth",
            "port a modpack",
            "export",
            "versions",
            "console",
            "screenshot",
            "shop",
            "wardrobe",
            "cosmetic",
            "agent",
            "instance",
        ] {
            assert!(labels(&entries, gone).is_empty(), "{gone}");
            assert!(
                !places.iter().any(|p| p.to_lowercase().contains(gone)),
                "{gone}"
            );
        }
    }

    /// A key as the window sends it, the field reading it without a clipboard.
    fn press(search: &mut Search, key: &str) -> Press {
        let typed = (key.chars().count() == 1).then_some(key);
        search.press(key, |field| field.key(key, typed, false, false))
    }

    fn on(search: &Search) -> &str {
        &search.rows().nth(search.index).unwrap().label
    }

    #[test]
    fn arrows_move_across_groups_and_round_the_ends() {
        let mut search = Search::new(sample(), (None, 0));
        assert_eq!(on(&search), "Home");
        assert!(press(&mut search, "up") == Press::Stay);
        assert_eq!(
            on(&search),
            "Show or hide the sidebar",
            "up from the first is the last"
        );
        assert!(press(&mut search, "down") == Press::Stay);
        assert_eq!(on(&search), "Home");
        for _ in 0..7 {
            press(&mut search, "down");
        }
        assert_eq!(on(&search), "Sunrise", "from the pages into the profiles");
        assert_eq!(search.query.text(), "", "the arrows are not typed");
    }

    #[test]
    fn typing_lists_again_from_the_first_row_and_enter_opens_the_row_the_keyboard_is_on() {
        let mut search = Search::new(sample(), (None, 0));
        press(&mut search, "down");
        for key in ["s", "e", "r"] {
            assert!(press(&mut search, key) == Press::Stay);
        }
        assert_eq!(search.query.text(), "ser");
        assert_eq!(on(&search), "Servers › Join", "the first row again");
        press(&mut search, "down");
        assert!(press(&mut search, "enter") == Press::Open(Target::Page(Page::Hosting)));
        // Backspace is the field's too, and lists again.
        press(&mut search, "backspace");
        assert_eq!(search.query.text(), "se");
        assert_eq!(search.index, 0);
    }

    #[test]
    fn escape_closes_tab_stays_and_enter_on_nothing_does_nothing() {
        let mut search = Search::new(sample(), (None, 0));
        assert!(press(&mut search, "tab") == Press::Stay);
        for key in ["q", "q", "q"] {
            press(&mut search, key);
        }
        assert!(search.rows().next().is_none());
        assert!(press(&mut search, "enter") == Press::Stay);
        assert!(press(&mut search, "down") == Press::Stay);
        assert!(press(&mut search, "escape") == Press::Close);
    }
}
