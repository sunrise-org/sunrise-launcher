//! Packs › Discover: resource, shader and data packs from Modrinth, installed through the
//! browser's checked path (`browser::installed`: checksum before anything is put in place).
//!
//! **Scope.** Only packs of the chosen kind: no mods or modpacks come from here, and a hit of
//! any other kind is dropped ([`keep`]).
//!
//! **Where a pack goes.** A resource or shader pack goes into the selected setup's folder. A
//! data pack goes into the worlds the player ticks in the world picker (`data_packs`): there is
//! no default world. Each world is checked again in the worker, under the setup's lock
//! ([`check_world`]).
//!
//! **Nothing else changes.** Installing never turns a pack on, starts the game or edits options;
//! it stays blocked while the game or another change runs, and holds Play and setup changes
//! through the shared busy flag (`mutating`).
use super::discover::MenuOption;
use super::state::Menu;
use super::ui::{added_mark, browse_card, c, card, columns, dim, muted};
use super::*;
use crate::paint::palette;
use crate::widgets::button::secondary;
use crate::widgets::chip::{self, choices, tag};
use gpui::SharedString;
use std::path::Path;
use sunrise_launcher::browser::details::{category_label, Sort};
use sunrise_launcher::browser::{self, images, installed, Kind, Project, Query, Source as _};
use sunrise_launcher::worlds;

/// How many hits one search asks for.
const LIMIT: usize = 24;
/// The longest search text sent.
const TEXT_MAX: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(super) enum Scope {
    #[default]
    Resource,
    /// Iris-format zips from Modrinth.
    Shader,
    Data,
}

impl Scope {
    pub(super) const ALL: [Scope; 3] = [Scope::Resource, Scope::Shader, Scope::Data];

    pub(super) fn label(self) -> &'static str {
        match self {
            Scope::Resource => tr("Resource packs"),
            Scope::Data => tr("Data packs"),
            Scope::Shader => tr("Shader packs"),
        }
    }

    /// The kind searched for.
    fn kind(self) -> Kind {
        match self {
            Scope::Resource => Kind::ResourcePack,
            Scope::Data => Kind::DataPack,
            Scope::Shader => Kind::Shader,
        }
    }
}

/// The filter row's choices, the same three Mods › Discover has.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct PackFilters {
    /// One of the scope's categories, or all.
    pub(super) category: Option<String>,
    /// Off: only packs made for the game Sunrise plays. On: every pack of the kind, each marked
    /// with how well it runs.
    pub(super) show_all: bool,
    /// None: by relevance with search text, else by downloads.
    pub(super) sort: Option<Sort>,
}

pub(super) enum Status {
    Idle,
    Searching,
    Done,
    Failed(String),
}

pub(super) struct PackState {
    pub(super) scope: Scope,
    pub(super) filters: PackFilters,
    /// The categories Modrinth offers for the scope, once they have been read.
    pub(super) categories: Option<(Scope, Vec<String>)>,
    pub(super) results: Vec<Project>,
    pub(super) status: Status,
    /// The number of the latest search: only its answer is taken.
    pub(super) seq: u64,
    /// The world "Find data packs" was pressed for: the picker opens with it ticked.
    pub(super) hint: Option<PathBuf>,
    /// The grid keeps its place while a pack's page is open.
    pub(super) scroll: gpui::ScrollHandle,
    /// The pack whose page is open.
    pub(super) open: Option<super::pack_project::PackOpen>,
    /// The "?" beside what Sunrise runs of a data pack: under the pointer, pressed open.
    help: (bool, bool),
}

impl Default for PackState {
    fn default() -> Self {
        Self {
            scope: Scope::Resource,
            filters: PackFilters::default(),
            categories: None,
            results: Vec::new(),
            status: Status::Idle,
            seq: 0,
            hint: None,
            scroll: gpui::ScrollHandle::new(),
            open: None,
            help: (false, false),
        }
    }
}

/// The restricted Modrinth query for `scope`: its one kind, what runs on `game` unless Show all
/// is on, the chosen category and sort, a bounded text and page.
pub(super) fn query(scope: Scope, text: &str, game: &str, filters: &PackFilters) -> Query {
    let mut q = Query::new(text, scope.kind());
    q.text = q.text.chars().take(TEXT_MAX).collect();
    q.game = game.to_owned();
    q.everything = filters.show_all;
    q.filters.categories = filters.category.iter().cloned().collect();
    q.filters.sort = filters.sort;
    q.limit = LIMIT;
    q
}

/// The hits a scope shows: Modrinth's, of the scope's kind only (a mod, modpack or other kind
/// that a source answered anyway is dropped), each project once.
pub(super) fn keep(scope: Scope, hits: Vec<Project>) -> Vec<Project> {
    let kind = scope.kind();
    let mut seen = std::collections::HashSet::new();
    hits.into_iter()
        .filter(|p| p.kind == kind && p.source == SourceId::Modrinth)
        .filter(|p| seen.insert((p.source, p.id.clone())))
        .collect()
}

/// Whether `world` is, right now, an existing world of the setup whose game folder is `home`:
/// a real folder directly in `saves/`, with neither it, `saves/` nor `datapacks/` a link to
/// somewhere else. Gives the world's name.
pub(super) fn check_world(home: &Path, world: &Path) -> Result<String, String> {
    let stale = || tr("That world is no longer in this profile.");
    let plain_dir = |p: &Path| {
        std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
    };
    let saves = home.join("saves");
    if world.parent() != Some(saves.as_path()) || !plain_dir(&saves) || !plain_dir(world) {
        return Err(stale().to_string());
    }
    let datapacks = world.join("datapacks");
    match std::fs::symlink_metadata(&datapacks) {
        Ok(m) if !m.is_dir() || m.file_type().is_symlink() => {
            return Err(tr("This world's datapacks folder isn't a plain folder.").to_string());
        }
        _ => {}
    }
    worlds::list(home)
        .into_iter()
        .find(|w| w.path == world)
        .map(|w| w.name)
        .ok_or_else(|| stale().to_string())
}

impl Launcher {
    /// The selected setup that packs may go into: a Sunrise one, with its folder readable.
    fn pack_setup(&self) -> Option<(String, PathBuf)> {
        self.selected_instance()
            .filter(|_| !self.location_unavailable)
            .map(|i| (i.id.clone(), i.home.clone()))
    }

    /// Switches Discover to `scope`.
    pub(super) fn pack_scope(&mut self, scope: Scope) {
        let p = &mut self.mods_state.packs;
        if p.scope == scope && !matches!(p.status, Status::Idle) {
            return;
        }
        p.scope = scope;
        // Each kind has its own categories.
        p.filters.category = None;
        p.hint = None;
        p.results.clear();
        p.status = Status::Idle;
        p.seq += 1;
        self.mods_state.menu = None;
        self.pack_search();
    }

    /// "Find data packs" on a world: Discover's data packs, with that world ticked in the
    /// picker. Changes no file.
    pub(super) fn find_data_packs(&mut self, world: Option<PathBuf>) {
        self.choose_view(Page::PackFind);
        self.pack_scope(Scope::Data);
        self.mods_state.packs.hint = world;
    }

    /// The setup changed: nothing chosen for the old one carries over.
    pub(super) fn pack_setup_moved(&mut self) {
        let p = &mut self.mods_state.packs;
        p.hint = None;
        p.open = None;
        p.results.clear();
        p.status = Status::Idle;
        p.seq += 1;
    }

    /// Searches Modrinth for the scope's kind with the header's text, in a worker.
    pub(super) fn pack_search(&mut self) {
        let text = self.mods_state.find.text().to_owned();
        let game = self.mods_game();
        let p = &mut self.mods_state.packs;
        let q = query(p.scope, &text, &game, &p.filters);
        p.seq += 1;
        p.status = Status::Searching;
        let read_categories = p.categories.as_ref().is_none_or(|(s, _)| *s != p.scope);
        let (seq, scope, tx) = (p.seq, p.scope, self.mods_state.tx.clone());
        std::thread::spawn(move || {
            let result = browser::modrinth::Modrinth
                .search(&q)
                .map(|hits| keep(scope, hits));
            let _ = tx.send(Event::Packs(seq, result));
            if read_categories {
                // Without them the menu offers only "All categories".
                let found = browser::modrinth::categories(scope.kind()).unwrap_or_default();
                let _ = tx.send(Event::PackCategories(scope, found));
            }
        });
    }

    pub(super) fn pack_categories(&mut self, scope: Scope, found: Vec<String>) {
        self.mods_state.packs.categories = Some((scope, found));
    }

    /// The filter row, the one Mods › Discover draws: a category of the scope, Show all, sort.
    /// Each choice searches again.
    fn pack_filter_bar(&self, cx: &mut Context<Self>) -> Div {
        let p = &self.mods_state.packs;
        let f = p.filters.clone();
        let categories = match &p.categories {
            Some((scope, found)) if *scope == p.scope => found.clone(),
            _ => Vec::new(),
        };
        let category_now = f
            .category
            .as_deref()
            .map_or_else(|| tr("All categories").to_owned(), category_label);
        let category_options = || -> Vec<MenuOption> {
            let pick = |category: Option<String>| -> Box<dyn Fn(&mut Launcher)> {
                Box::new(move |this: &mut Launcher| {
                    this.mods_state.packs.filters.category = category.clone();
                    this.pack_search();
                })
            };
            std::iter::once((
                tr("All categories").into(),
                f.category.is_none(),
                pick(None),
            ))
            .chain(categories.iter().map(|k| {
                (
                    category_label(k).into(),
                    f.category.as_ref() == Some(k),
                    pick(Some(k.clone())),
                )
            }))
            .collect()
        };
        let sort_now = browser::details::Filters {
            sort: f.sort,
            ..Default::default()
        }
        .sort(self.mods_state.find.text());
        let sort_options = || -> Vec<MenuOption> {
            Sort::ALL
                .into_iter()
                .map(|s| {
                    (
                        tr(s.label()).into(),
                        sort_now == s,
                        Box::new(move |this: &mut Launcher| {
                            this.mods_state.packs.filters.sort = Some(s);
                            this.pack_search();
                        }) as Box<dyn Fn(&mut Launcher)>,
                    )
                })
                .collect()
        };
        let category_menu = self.discover_menu(&category_now, Menu::Category, category_options, cx);
        let sort_menu = self.discover_menu(tr(sort_now.label()), Menu::Sort, sort_options, cx);
        self.filter_bar(
            category_menu,
            f.show_all,
            |this| {
                let filters = &mut this.mods_state.packs.filters;
                filters.show_all = !filters.show_all;
                this.pack_search();
            },
            sort_menu,
            cx,
        )
    }

    pub(super) fn pack_answer(&mut self, seq: u64, result: Result<Vec<Project>, String>) {
        let p = &mut self.mods_state.packs;
        if seq != p.seq {
            return;
        }
        match result {
            Ok(found) => {
                p.results = found;
                p.status = Status::Done;
            }
            Err(e) => {
                p.results.clear();
                p.status = Status::Failed(e);
            }
        }
    }

    /// A pack install ended: its words; what lists packs is read again.
    pub(super) fn pack_installed(&mut self, result: Result<String, String>) {
        self.mods_state.busy = None;
        self.mods_state.notice = Some(result);
        self.game_packs.stale();
        self.world_packs.reread();
        self.mods_state.data.reread();
    }

    /// Add on a result: a data pack asks which worlds; a resource or shader pack is checked
    /// against the scope and the selected setup, then installed in a worker.
    pub(super) fn pack_install(&mut self, source: SourceId, id: &str) {
        let scope = self.mods_state.packs.scope;
        let Some(project) = self
            .mods_state
            .packs
            .results
            .iter()
            .find(|p| p.source == source && p.id == id && p.kind == scope.kind())
            .cloned()
        else {
            return;
        };
        if project.kind == Kind::DataPack {
            let hint = self.mods_state.packs.hint.clone();
            self.pick_worlds(super::data_packs::What::Project(project), hint);
            return;
        }
        if !self.guard() {
            return;
        }
        let Some((_, home)) = self.pack_setup() else {
            self.mods_state.notice = Some(Err(tr("Select a Sunrise profile first.").to_string()));
            return;
        };
        self.dismiss_notice();
        self.mods_state.busy = Some((tr("Checking the pack…").into(), 0.));
        let (tx, game) = (self.mods_state.tx.clone(), self.mods_game());
        std::thread::spawn(move || {
            let result = install(&project, &home, &game, &|line, f| {
                let _ = tx.send(Event::Progress(line.to_owned(), f));
            });
            let _ = tx.send(Event::PackDone(result));
        });
    }

    /// Where data packs come from and how much of one Sunrise runs, said once for every card:
    /// the short of it, and the whole list behind a "?".
    fn data_pack_line(&self, cx: &mut Context<Self>) -> AnyElement {
        let (over, pinned) = self.mods_state.packs.help;
        let help = div()
            .id("alt-data-help")
            .tab_index(0)
            .seen()
            .flex_none()
            .size(px(16.))
            .rounded_full()
            .border_1()
            .border_color(c(if pinned {
                palette().text
            } else {
                palette().line
            }))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(tokens::CAPTION))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(c(if pinned {
                palette().text
            } else {
                palette().muted
            }))
            .cursor_pointer()
            .focus_visible(|s| s.border_color(c(palette().focus)))
            // One line the size of the ring's inside, its text centred: the "?" sits in the
            // middle both ways, whatever width its text element measures.
            .child(
                div()
                    .w_full()
                    .text_center()
                    .line_height(px(16. - 2.))
                    .child("?"),
            )
            .on_hover(cx.listener(|this, over: &bool, _, cx| {
                this.mods_state.packs.help.0 = *over;
                cx.notify();
            }))
            // A press pins it open, for keyboards and touch; another press closes it.
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                let pinned = &mut this.mods_state.packs.help.1;
                *pinned = !*pinned;
                cx.notify();
            }));
        let said = (over || pinned).then(|| {
            gpui::deferred(
                div()
                    .absolute()
                    .top(px(24.))
                    .right_0()
                    .w(px(360.))
                    .max_w(px(self.content_width()))
                    .p(px(12.))
                    .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                    .border_1()
                    .border_color(palette().input_edge())
                    .bg(c(palette().raised))
                    .shadow_lg()
                    .occlude()
                    .child(muted(browser::datapack_support())),
            )
            .with_priority(10)
        });
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(muted(tr(
                "From Modrinth, into the worlds you choose. Sunrise runs most of what a data pack does.",
            )))
            .child(div().relative().flex_none().child(help).children(said))
            .into_any_element()
    }

    /// Packs › Discover: the header Mods has, the three kinds, where they come from, the cards.
    pub(super) fn pack_find(
        &mut self,
        window: &mut Window,
        focus: &FocusHandle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if matches!(self.mods_state.packs.status, Status::Idle) {
            self.pack_search();
        }
        let scope = self.mods_state.packs.scope;
        let options: Vec<(Scope, SharedString)> = Scope::ALL
            .into_iter()
            .map(|kind| (kind, kind.label().into()))
            .collect();
        let this = cx.entity();
        let kinds = choices("alt-discover-scope", &options, scope, move |kind, _, cx| {
            this.update(cx, |this, cx| {
                this.pack_scope(kind);
                cx.notify();
            })
        })
        .gap(px(8.));
        let from: AnyElement = match scope {
            Scope::Data => self.data_pack_line(cx),
            Scope::Shader => muted(tr(
                "From Sunrise's registry and Modrinth, into this profile's shader packs.",
            ))
            .into_any_element(),
            Scope::Resource => {
                muted(tr("From Modrinth, into this profile's resource packs.")).into_any_element()
            }
        };
        if scope == Scope::Data {
            self.data_list(cx);
        }
        let filters = self.pack_filter_bar(cx);
        let page = self
            .discover_shell(window, focus, cx)
            .child(kinds)
            .child(from)
            .child(filters);
        let added: HashSet<(SourceId, String)> = self
            .mods_state
            .snapshot
            .as_ref()
            .map(|s| {
                s.entries
                    .iter()
                    .map(|e| (e.source, e.project.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let working = self.mods_state.busy.is_some();
        let results = self.mods_state.packs.results.clone();
        let body: AnyElement = match &self.mods_state.packs.status {
            Status::Searching | Status::Idle => muted(tr("Searching…")).into_any_element(),
            Status::Failed(e) => card()
                .p(px(20.))
                .flex()
                .flex_col()
                .items_start()
                .gap(px(10.))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(tr("Modrinth can't be reached")),
                )
                .child(muted(e.clone()))
                .child(
                    secondary("alt-pack-retry", tr("Try again")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.pack_search();
                            cx.notify();
                        },
                    )),
                )
                .into_any_element(),
            Status::Done if results.is_empty() => {
                muted(tr("Nothing found. Try another search.")).into_any_element()
            }
            Status::Done => {
                let game = self.mods_game();
                let cols = columns(self.content_width()).max(1) as usize;
                let row_count = results.len().div_ceil(cols);
                let cull = self.mods.pack_discover_list.clone();
                cull.items({
                    use std::hash::{Hash, Hasher};
                    let mut hash = std::collections::hash_map::DefaultHasher::new();
                    cols.hash(&mut hash);
                    for p in &results {
                        (p.source as u8, p.id.as_str()).hash(&mut hash);
                    }
                    hash.finish()
                });
                let scroll = self.mods_state.packs.scroll.clone();
                cull.column(
                    &scroll,
                    row_count,
                    12.,
                    |_| false,
                    |row_idx| {
                        let start = row_idx * cols;
                        let end = (start + cols).min(results.len());
                        let mut row = div().grid().grid_cols(cols as u16).gap(px(12.));
                        for (offset, p) in results[start..end].iter().enumerate() {
                            let n = start + offset;
                            let picture = p
                                .icon
                                .clone()
                                .and_then(|url| self.mods_image(&url, images::ICON));
                            let support = p.support(&game);
                            let data = p.kind == Kind::DataPack;
                            let is_added = added.contains(&(p.source, p.id.clone())) && !data;
                            let control = if is_added {
                                Some(added_mark())
                            } else if !support.installable() {
                                None
                            } else {
                                // A data pack asks which worlds first.
                                let label = if data { tr("Add…") } else { tr("Add") };
                                let b = secondary(format!("alt-pack-add-{}", n), label);
                                Some(if working {
                                    dim(b).into_any_element()
                                } else {
                                    let id = p.id.clone();
                                    let source = p.source;
                                    b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        this.pack_install(source, &id);
                                        cx.stop_propagation();
                                        cx.notify();
                                    }))
                                    .into_any_element()
                                })
                            };
                            // How well it runs here, in the source's words. What Sunrise runs of any
                            // data pack is said once, above the cards.
                            let common =
                                data && support.reason() == Some(browser::datapack_support());
                            let note = support.reason().filter(|_| !common).map(str::to_owned);
                            let mut facts = vec![tag(source_label(p.source), chip::Kind::Neutral)];
                            if !matches!(support, browser::Support::Works) && !common {
                                facts.push(tag(support.label(), chip::Kind::Neutral));
                            }
                            facts.push(muted(model::downloads(p.downloads)));
                            if data {
                                match self.mods_state.data.worlds_with(p.source, &p.id) {
                                    0 => {}
                                    1 => facts.push(muted(tr("In 1 world"))),
                                    n => facts.push(muted(trf(
                                        "In {count} worlds",
                                        &[("count", &n.to_string())],
                                    ))),
                                }
                            }
                            let (source, id) = (p.source, p.id.clone());
                            row = row.child(
                                browse_card(
                                    ElementId::named_usize("alt-pack-card", n),
                                    picture,
                                    &id,
                                    &p.title,
                                    &p.author,
                                    &p.summary,
                                    facts,
                                    control,
                                    note,
                                )
                                .on_click(cx.listener(
                                    move |this, _: &ClickEvent, _, cx| {
                                        this.pack_show(source, &id);
                                        cx.notify();
                                    },
                                )),
                            );
                        }
                        row
                    },
                )
                .into_any_element()
            }
        };
        crate::frame::scrolls_with(&self.mods_state.packs.scroll);
        page.child(crate::scrollbar::wrap_handle(
            "pack-discover",
            self.mods_state.packs.scroll.clone(),
            div()
                .id("alt-pack-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(body),
            &self.theme,
        ))
        .into_any_element()
    }
}

/// The words a card and a pack's page name a source by.
pub(super) fn source_label(source: SourceId) -> &'static str {
    match source {
        SourceId::Sunrise => tr("Sunrise registry"),
        other => other.label(),
    }
}

/// Puts a resource or shader pack in place with the browser's checked install, under the
/// setup's lock. Nothing is turned on.
fn install(
    project: &Project,
    home: &Path,
    game: &str,
    progress: &dyn Fn(&str, f32),
) -> Result<String, String> {
    let source = browser::modrinth::Modrinth;
    let _lock = browser::try_lock_home(home).ok_or_else(|| {
        tr("This profile is busy. Try again when the operation finishes.").to_string()
    })?;
    match project.kind {
        Kind::ResourcePack | Kind::Shader => {
            let done = installed::install(&source, project, false, home, game, progress)?;
            let names: Vec<String> = done
                .iter()
                .map(|e| format!("{} {}", e.title, e.number))
                .collect();
            Ok(trf(
                "Installed {mods}, checksums checked. Turn it on under Installed.",
                &[("mods", &names.join(", "))],
            ))
        }
        _ => Err(tr("Only resource, data and shader packs install from here.").to_string()),
    }
}

#[cfg(test)]
mod tests;
