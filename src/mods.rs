//! Mods: Discover, Installed and a mod's page.
//!
//! **Real:** the registry list, search, categories, fit, kind and sort (`browser::sunrise`'s index,
//! read in a worker); a mod's page from the index (versions and their fit, size, licence,
//! authors, source, mirrored screenshots, what it may do); Add and single Update through the
//! existing install path (native code asks for permission first, `native_consent`); Installed
//! from `mod-browser.json` and `sunrise-mods.txt` with on/off, selection, filters, bulk Turn on,
//! Turn off, Remove and Update N mods (a reviewed plan, applied by a worker); Build… and Allow…
//! hand over to the one native build path.
//!
//! **Not here, said on screen or in the handoff:** Featured (the registry carries no editorial
//! list), a version picker (the registry installs the newest that fits), dependency lists for
//! plain mods (the index lists a mod's needs for packs only), "Add from a file…", resource,
//! shader and data packs, and registry modpacks.
//!
//! **Writes** happen only for an action the player took: install, update, on/off, remove. Showing
//! the screens reads the index (its own cache refresh, in a worker), `mod-browser.json`,
//! `sunrise-mods.txt` and file times. Nothing turns off or repairs a mod while drawing: taking
//! down what the registry revoked is left to `before_play` and the row says so.
use super::*;
use crate::frame::Page;
use crate::frame::Reach as _;
use crate::line_edit::{self, Key, LineEdit};
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use sunrise_launcher::browser::{
    installed::{self, Entry},
    mod_list, sunrise, Source, SourceId,
};

mod actions;
mod build_modal;
pub(crate) mod build_run;
mod data_packs;
mod discover;
mod installed_view;
mod model;
mod pack_discover;
mod pack_project;
mod packs;
mod project;
mod search;
mod ui;
pub(crate) use search::Installed;
mod viewer;
use std::rc::Rc;
pub(crate) use ui::{checkbox, choices, field_theme, reveal, segmented, switch, Choose};

use crate::paint::palette;
use model::{Card, Row};

#[derive(Clone, Copy, PartialEq)]
enum Menu {
    Category,
    Make,
    Sort,
}

enum Catalog {
    Idle,
    Loading,
    Ready(Vec<Card>, Vec<String>),
    Failed(String),
}

/// A batch the player asked for and has to confirm: named, counted, nothing started yet.
enum Confirm {
    /// (mod id, project id, title)
    Remove(Vec<(String, String, String)>),
    Update(model::UpdatePlan),
}

/// Inputs that can change the Installed row model independently of scrolling or selection.
struct RowsMemo {
    snapshot_key: String,
    index: Option<std::sync::Arc<sunrise_launcher_smod::index::Index>>,
    version: Option<String>,
    kept: String,
    native_allowed: bool,
    rows: Rc<Vec<Row>>,
}

struct Open {
    id: String,
    /// The screen Back returns to.
    from: Page,
    page: Option<Result<Box<model::Page>, String>>,
}

struct Snapshot {
    key: String,
    home: PathBuf,
    entries: Vec<Entry>,
    enabled: HashSet<String>,
    chosen: std::collections::BTreeMap<String, String>,
    /// Mod files the registry didn't install: on/off is editable, the files are not.
    local: Vec<model::Local>,
}

enum Event {
    Catalog(Result<(Vec<Card>, Vec<String>), String>),
    Page(String, Result<Box<model::Page>, String>),
    Progress(String, f32),
    /// A pack search's answer, by the number of the search that asked.
    Packs(u64, Result<Vec<sunrise_launcher::browser::Project>, String>),
    /// A pack's page, by its source and id.
    PackPage((SourceId, String), pack_project::PackPage),
    /// A resource or data pack install ended: what it said.
    PackDone(Result<String, String>),
    /// The batch was refused before changing anything.
    Refused(String),
    /// A batch ended: what it said, and which mods failed and why.
    Done(String, Vec<(String, String)>),
    /// "Check for update" asked the registry again: the mod's name, and the newer version found.
    Checked(String, Result<Option<String>, String>),
}

/// The kinds of pack Packs › Installed lists, one at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shelf {
    Resource,
    Shader,
    /// Data packs, which live in worlds (`data_packs`).
    Data,
}

impl Shelf {
    /// The setup's own collection it lists; none for data packs.
    fn kind(self) -> Option<sunrise_launcher::packs::Kind> {
        match self {
            Shelf::Resource => Some(sunrise_launcher::packs::Kind::Resource),
            Shelf::Shader => Some(sunrise_launcher::packs::Kind::Shader),
            Shelf::Data => None,
        }
    }
}

pub(crate) struct ModsState {
    tx: Sender<Event>,
    rx: Receiver<Event>,
    catalog: Catalog,
    find: LineEdit,
    find_focus: Option<FocusHandle>,
    have: LineEdit,
    have_focus: Option<FocusHandle>,
    filters: model::Filters,
    menu: Option<Menu>,
    row_filter: model::RowFilter,
    /// The kind Packs › Installed shows.
    shelf: Shelf,
    /// Discover's resource and data pack scopes (`pack_discover`).
    packs: pack_discover::PackState,
    /// The worlds' data packs and the world picker (`data_packs`).
    data: data_packs::DataState,
    selection: BTreeSet<String>,
    /// The stable ID of the last selected row, for Shift ranges even after list reordering.
    anchor: Option<String>,
    installed_rows: std::cell::RefCell<Option<RowsMemo>>,
    installed_view: installed_view::InstalledView,
    confirm: Option<Confirm>,
    /// The folder the open confirmation was made for.
    reviewed_home: Option<PathBuf>,
    snapshot: Option<Snapshot>,
    open: Option<Open>,
    /// My own batch (update or remove): its line and fraction.
    busy: Option<(String, f32)>,
    notice: Option<Result<String, String>>,
    /// Why a mod's last update failed, shown on its row until the next attempt.
    errors: HashMap<String, String>,
    /// How many cards Discover draws ("Show more" adds a page).
    limit: usize,
    /// The Installed row whose "?" is open (a mod not on this build), and whether its
    /// technical details show.
    unfit_open: Option<String>,
    unfit_details: bool,
    /// The build the build window shows: running, or ended and not yet closed.
    pub(crate) build: Option<build_run::Run>,
    /// Build was pressed and the build has not started yet (it asks for permission first).
    pub(crate) build_wanted: bool,
    /// Whether Installed drew its Build button in the frame being drawn (the page is drawn
    /// before the status strip).
    pub(crate) build_button_drawn: bool,
    /// Whether a mod page's Details (who made it, how it is checked) is open.
    facts_open: bool,
    /// Discover or Installed, as the player last chose on Mods and on Packs this session.
    last_mods: Option<Page>,
    last_packs: Option<Page>,
    /// The row another page opened Installed at, and whether it was scrolled to yet.
    reveal: Option<search::Reveal>,
    revealed: Rc<std::cell::Cell<bool>>,
    /// A gallery picture enlarged over the window.
    viewer: Option<viewer::Viewer>,
}

impl Default for ModsState {
    fn default() -> Self {
        let (tx, rx) = crate::wake::channel();
        Self {
            tx,
            rx,
            catalog: Catalog::Idle,
            find: LineEdit::default(),
            find_focus: None,
            have: LineEdit::default(),
            have_focus: None,
            filters: model::Filters::default(),
            menu: None,
            row_filter: model::RowFilter::default(),
            shelf: Shelf::Resource,
            packs: Default::default(),
            data: Default::default(),
            selection: BTreeSet::new(),
            anchor: None,
            installed_rows: Default::default(),
            installed_view: Default::default(),
            confirm: None,
            reviewed_home: None,
            snapshot: None,
            open: None,
            busy: None,
            notice: None,
            errors: HashMap::new(),
            limit: 0,
            unfit_open: None,
            unfit_details: false,
            build: None,
            build_wanted: false,
            build_button_drawn: false,
            facts_open: false,
            last_mods: None,
            last_packs: None,
            reveal: None,
            revealed: Default::default(),
            viewer: None,
        }
    }
}

/// The read-only Home view uses the same Installed classification as the full rows.
pub(crate) struct HomeModRows(std::collections::HashMap<String, Row>);

pub(crate) struct HomeModFacts {
    pub(crate) kind: &'static str,
    pub(crate) status: Option<String>,
}

impl Launcher {
    /// The Mods screen for `self.ui.page`: Discover, Installed or a mod's page.
    pub(crate) fn mods(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.mods_drain();
        self.mods_refresh();
        if self.ui.page == Page::Packs {
            return self.pack_page(window, cx);
        }
        let m = &mut self.ui.mods;
        let find_focus = m
            .find_focus
            .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        let have_focus = m
            .have_focus
            .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        match self.ui.page {
            Page::Project => self.project(cx),
            Page::Installed => self.installed(window, &have_focus, cx),
            Page::PackFind => self.pack_find(window, &find_focus, cx),
            Page::PackProject => self.pack_project(cx),
            _ => self.discover(window, &find_focus, cx),
        }
    }

    /// Opens a mod's page, from Home or anywhere else. Its details are read in a worker.
    pub(crate) fn mods_show_project(&mut self, id: String) {
        let from = match self.ui.page {
            Page::Project => self
                .ui
                .mods
                .open
                .as_ref()
                .map_or(Page::Discover, |o| o.from),
            Page::Home | Page::Installed => self.ui.page,
            _ => Page::Discover,
        };
        self.ui.page = Page::Project;
        self.mods_load_page(id, from);
    }

    /// The installed mods `ids` as people read them: each one's name, and the newer version the
    /// registry has for it when there is one. Reads what is already in memory.
    pub(crate) fn mods_named(&self, ids: &[String]) -> Vec<(String, Option<String>)> {
        let rows = self.rows();
        ids.iter()
            .map(|id| match rows.iter().find(|r| &r.id == id) {
                Some(row) => (row.title.clone(), row.update.clone()),
                None => (id.clone(), None),
            })
            .collect()
    }

    /// The native mods turned on for the selected setup, as a build's lines name them: id,
    /// title, version and picture.
    pub(crate) fn mods_lines(&mut self) -> Vec<(String, String, String, Option<String>)> {
        self.mods_refresh();
        let rows = self.rows();
        native_status::enabled(&self.mods_home())
            .into_iter()
            .map(|(id, version, _)| match rows.iter().find(|r| r.id == id) {
                Some(row) => (id, row.title.clone(), version, row.icon.clone()),
                None => (id.clone(), id, version, None),
            })
            .collect()
    }

    /// Updates the installed mod `id` to the newer version the registry has, as its row does.
    pub(crate) fn update_mod(&mut self, id: &str) {
        if let Some(row) = self.rows().into_iter().find(|r| r.id == id) {
            self.update_one(&row);
        }
    }

    pub(crate) fn home_mod_rows(&mut self) -> HomeModRows {
        self.mods_refresh();
        HomeModRows(
            self.rows()
                .into_iter()
                .filter(|row| row.on)
                .map(|row| (row.id.clone(), row))
                .collect(),
        )
    }

    pub(crate) fn home_mod_facts(&self, rows: &HomeModRows, id: &str) -> Option<HomeModFacts> {
        let row = rows.0.get(id)?;
        let error = self.ui.mods.errors.get(id).map(String::as_str);
        let building = self
            .build_here()
            .and_then(|run| run.state_of(id, &row.version));
        let cells = model::cells(row, error, building);
        let kind = if cells.kind.is_empty() {
            tr("Content")
        } else {
            cells.kind
        };
        Some(HomeModFacts {
            kind,
            status: cells.status.map(|state| state.label),
        })
    }

    pub(crate) fn show_resource_pack_discover(&mut self) {
        self.leave_section(true);
        self.ui.mods.shelf = Shelf::Resource;
        self.choose_view(Page::PackFind);
    }

    /// The play bar's quiet line about them: Mods › Installed, where each says why.
    pub(crate) fn show_installed_mods(&mut self) {
        self.leave_section(false);
        self.mods_refresh();
        self.choose_view(Page::Installed);
    }

    /// Mods › Discover itself, whatever Mods last showed.
    pub(crate) fn show_discover(&mut self) {
        self.leave_section(false);
        self.choose_view(Page::Discover);
    }

    /// The rail's Mods: where the player last was this session, else Installed when this setup
    /// has mods and Discover when it has none.
    pub(super) fn open_mods(&mut self) {
        self.leave_section(false);
        self.ui.page = self.ui.mods.last_mods.unwrap_or_else(|| {
            self.mods_refresh();
            match self.rows().is_empty() {
                true => Page::Discover,
                false => Page::Installed,
            }
        });
    }

    /// The rail's Packs, by the same rule as [`Self::open_mods`].
    pub(super) fn open_packs(&mut self) {
        self.leave_section(true);
        self.ui.page = self.ui.mods.last_packs.unwrap_or_else(|| {
            self.load_packs_list(self.mods_home());
            match self.game_packs.resource.is_empty() && self.game_packs.shader.is_empty() {
                true => Page::PackFind,
                false => Page::Packs,
            }
        });
    }

    /// Mods and Packs share their search fields: going from one to the other starts them empty.
    fn leave_section(&mut self, to_packs: bool) {
        let in_packs = matches!(
            self.ui.page,
            Page::Packs | Page::PackFind | Page::PackProject
        );
        let in_mods = matches!(
            self.ui.page,
            Page::Discover | Page::Installed | Page::Project
        );
        if (to_packs && in_packs) || (!to_packs && in_mods) {
            return;
        }
        self.reset_pack_view();
        let m = &mut self.ui.mods;
        m.find.set(String::new());
        m.filters.text.clear();
        m.menu = None;
        m.packs.hint = None;
        m.data = Default::default();
    }

    /// Discover or Installed, chosen on the header's switch: remembered for the session.
    fn choose_view(&mut self, page: Page) {
        let m = &mut self.ui.mods;
        match page {
            Page::Discover | Page::Installed => m.last_mods = Some(page),
            _ => m.last_packs = Some(page),
        }
        m.selection.clear();
        m.anchor = None;
        m.confirm = None;
        m.menu = None;
        m.reveal = None;
        self.game_packs.confirm_remove = None;
        // Packs keeps the kind across the switch.
        match page {
            Page::PackFind => self.pack_scope(match self.ui.mods.shelf {
                Shelf::Resource => pack_discover::Scope::Resource,
                Shelf::Shader => pack_discover::Scope::Shader,
                Shelf::Data => pack_discover::Scope::Data,
            }),
            Page::Packs => {
                self.ui.mods.shelf = match self.ui.mods.packs.scope {
                    pack_discover::Scope::Resource => Shelf::Resource,
                    pack_discover::Scope::Shader => Shelf::Shader,
                    pack_discover::Scope::Data => Shelf::Data,
                };
                self.ui.mods.data.reread();
            }
            _ => {}
        }
        self.ui.page = page;
    }

    /// The width a page's own content has: the window less its edges, the sidebar (open or
    /// closed), the gap beside it, the pane's border and the page's margins.
    pub(super) fn content_width(&self) -> f32 {
        let edges = 2. * crate::tokens::FRAME_INSET + 12.;
        self.window_width
            - edges
            - crate::rail::rail_width(self.settings.sidebar_expanded)
            - 2.
            - 48.
    }

    fn mods_home(&self) -> PathBuf {
        self.selected_instance()
            .map(|i| i.home.clone())
            .unwrap_or_else(backend::game_home)
    }

    /// What the workers said. Nothing here touches a file.
    pub(super) fn mods_drain(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.ui.mods.rx.try_recv() {
            changed = true;
            let m = &mut self.ui.mods;
            match event {
                Event::Catalog(Ok((cards, categories))) => {
                    m.catalog = Catalog::Ready(cards, categories)
                }
                Event::Catalog(Err(e)) => m.catalog = Catalog::Failed(e),
                Event::Page(id, page) => {
                    if let Some(open) = m.open.as_mut().filter(|o| o.id == id) {
                        open.page = Some(page);
                    }
                }
                Event::Progress(line, f) => m.busy = Some((line, f)),
                Event::Packs(seq, result) => self.pack_answer(seq, result),
                Event::PackPage(key, page) => self.pack_page_answer(key, page),
                Event::PackDone(result) => self.pack_installed(result),
                Event::Checked(title, found) => self.checked(title, found),
                Event::Refused(why) => {
                    m.busy = None;
                    m.notice = Some(Err(why));
                }
                Event::Done(said, failed) => {
                    m.busy = None;
                    m.errors.clear();
                    m.errors.extend(failed.iter().cloned());
                    m.notice = Some(match failed.len() {
                        0 => Ok(said),
                        count => {
                            let failed_count =
                                trf("{count} failed", &[("count", &count.to_string())]);
                            let details = failed
                                .iter()
                                .map(|(id, why)| format!("{id}: {why}"))
                                .collect::<Vec<_>>()
                                .join("\n");
                            let (summary, rest) = said.split_once('\n').unwrap_or((&said, ""));
                            Err(format!("{summary} · {failed_count}\n{rest}\n{details}"))
                        }
                    });
                    // The list is read again by the next refresh (the files' times moved).
                }
            }
        }
        changed
    }

    /// Reads what is installed again when the files' fingerprint moved (an install, a toggle or a
    /// build elsewhere), and asks for the registry once. Reads only.
    fn mods_refresh(&mut self) {
        let home = self.mods_home();
        self.mods
            .follow_home(self.selected_instance().map(|i| i.home.clone()));
        let key = native_status::fingerprint(&home, None);
        let moved = self
            .ui
            .mods
            .snapshot
            .as_ref()
            .is_some_and(|s| s.home != home);
        if moved {
            // Another setup: nothing reviewed or selected for the old one carries over.
            let m = &mut self.ui.mods;
            m.confirm = None;
            m.reviewed_home = None;
            m.selection.clear();
            m.anchor = None;
            m.errors.clear();
            m.notice = None;
            self.pack_setup_moved();
        }
        if self.ui.mods.snapshot.as_ref().is_none_or(|s| s.key != key) {
            let entries = installed::Installed::load(&home).entries;
            let local = |native: bool| {
                move |f: native_status::Installed| model::Local {
                    id: f.id,
                    name: f.name,
                    version: f.version,
                    github: f.origin.is_some(),
                    native,
                }
            };
            let local: Vec<model::Local> = native_status::untracked(&home)
                .into_iter()
                .map(local(true))
                .chain(
                    native_status::untracked_wasm(&home)
                        .into_iter()
                        .map(local(false)),
                )
                .collect();
            let enabled = mod_list::enabled_ids(&home);
            let chosen = sunrise::chosen(&home);
            self.ui.mods.snapshot = Some(Snapshot {
                key,
                home,
                entries,
                enabled,
                chosen,
                local,
            });
            let rows: HashSet<String> = self.rows().into_iter().map(|r| r.id).collect();
            self.ui.mods.selection.retain(|id| rows.contains(id));
        }
        let wants_index = self
            .ui
            .mods
            .snapshot
            .as_ref()
            .is_some_and(|s| !s.entries.is_empty())
            || self.ui.page != Page::Installed;
        if wants_index {
            self.ensure_mod_catalog();
        }
    }

    pub(crate) fn ensure_mod_catalog(&mut self) {
        if !matches!(self.ui.mods.catalog, Catalog::Idle) {
            return;
        }
        self.ui.mods.catalog = Catalog::Loading;
        let tx = self.ui.mods.tx.clone();
        let version = self
            .selected_instance()
            .map(|i| self.version_of(i).cloned());
        std::thread::spawn(move || {
            let game = version.map_or_else(sunrise_launcher::browser::game, |v| {
                backend::played_version_id_by(v.as_deref())
            });
            let result = sunrise::index().map(|index| {
                let cards = model::cards(&index, &game);
                let categories = model::categories(&cards);
                (cards, categories)
            });
            let _ = tx.send(Event::Catalog(result));
        });
    }

    fn mods_load_page(&mut self, id: String, from: Page) {
        self.ui.mods.open = Some(Open {
            id: id.clone(),
            from,
            page: None,
        });
        let (tx, game) = (self.ui.mods.tx.clone(), self.mods_game());
        let instance = instances::selected();
        std::thread::spawn(move || {
            let result = sunrise::index().and_then(|index| {
                let trust = sunrise::trust_and_capabilities(&id, Some(&instance), &game);
                let another = sunrise::made_for_another_build(&id, &instance, &game);
                model::page(&index, &id, &game, trust, another)
                    .map(Box::new)
                    .ok_or_else(|| trf("{id} isn't a mod in Sunrise's registry.", &[("id", &id)]))
            });
            let _ = tx.send(Event::Page(id, result));
        });
    }

    /// The installed mods as rows. Uses only the index served over HTTPS this run for registry
    /// names and statuses. Never asks the network or writes.
    fn rows(&self) -> Vec<Row> {
        self.installed_rows().as_ref().clone()
    }

    fn installed_rows(&self) -> Rc<Vec<Row>> {
        let Some(snap) = &self.ui.mods.snapshot else {
            return Rc::default();
        };
        let index = sunrise::fetched_index();
        let version = self.selected_instance().and_then(|i| self.version_of(i));
        let kept = native_mods::kept_stamp(&snap.home);
        let native_allowed = self.settings.native_mods_allowed_at.is_some();
        let mut cache = self.ui.mods.installed_rows.borrow_mut();
        if let Some(memo) = cache.as_ref().filter(|memo| {
            let same_index = match (&index, &memo.index) {
                (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            };
            memo.snapshot_key == snap.key
                && memo.native_allowed == native_allowed
                && memo.version.as_ref() == version
                && memo.kept == kept
                && same_index
        }) {
            return memo.rows.clone();
        }
        let unbuilt: HashSet<String> = match !snap.local.is_empty()
            || snap
                .entries
                .iter()
                .any(|e| native_status::is_native_file(&e.file))
        {
            true => {
                self.native_mod_unbuilt("");
                self.mods
                    .native_state
                    .borrow()
                    .as_ref()
                    .map(|(_, _, ids)| ids.iter().cloned().collect())
                    .unwrap_or_default()
            }
            false => HashSet::new(),
        };
        let game = self.mods_game();
        let mut rows = model::rows(
            &snap.entries,
            &model::World {
                index: index.as_deref(),
                game: &game,
                enabled: &snap.enabled,
                unbuilt: &unbuilt,
                native_allowed,
                chosen: &snap.chosen,
                local: &snap.local,
            },
        );
        // A mod the game was made without isn't waiting for a build: it waits for its author.
        let left = self.native_left_out();
        if !left.is_empty() {
            let on = self
                .selected_instance()
                .and_then(|i| self.version_of(i))
                .map(|tag| crate::sheets::build_name(tag))
                .unwrap_or_default();
            model::mark_left_out(&mut rows, &left, &on, crate::sheets::build_name);
            // A row that no longer needs the player moves out of that group.
            model::sort(&mut rows);
        }
        let rows = Rc::new(rows);
        *cache = Some(RowsMemo {
            snapshot_key: snap.key.clone(),
            index,
            version: version.cloned(),
            kept,
            native_allowed,
            rows: rows.clone(),
        });
        rows
    }

    /// Whether another install or update holds the folder right now.
    fn mods_busy(&self) -> bool {
        self.mutating()
    }

    /// Whether a mod change is in progress (mine or the shared install's), for the shared Play
    /// and setup gates. Reads only the Mods busy flags, never the launch state.
    pub(super) fn mutating(&self) -> bool {
        self.ui.mods.busy.is_some() || self.mods.busy.is_some()
    }

    /// Build… on a row: the one native build path, which asks for permission first. The player
    /// stays where they are: the play bar shows how far it is.
    fn build(&mut self) {
        self.build_native();
    }

    /// Allow… on a row: the native-code question, with what it builds.
    fn allow(&mut self) {
        self.request_native_build_consent();
    }

    pub(super) fn open_url(url: &str, cx: &mut Context<Self>) {
        if url.starts_with("https://") {
            cx.open_url(url);
        }
    }

    /// What the notice strip says: the batch's own, else what the shared install path said.
    fn notice(&self) -> Option<Result<String, String>> {
        self.ui
            .mods
            .notice
            .clone()
            .or_else(|| self.mods.message.clone())
    }

    pub(super) fn dismiss_notice(&mut self) {
        self.ui.mods.notice = None;
        self.mods.message = None;
    }

    /// The strip shown above either list: progress, or the last result.
    fn strip(&self, cx: &mut Context<Self>) -> Option<Div> {
        use ui::{c, muted};
        let busy = self.ui.mods.busy.clone().or_else(|| {
            self.mods
                .busy
                .as_ref()
                .map(|(_, line, f)| (line.clone(), *f))
        });
        // A build is of mods: its strip is Mods' own. The play bar shows it on every page.
        if let Some((line, f)) = busy {
            return Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(div().text_size(px(tokens::LABEL)).truncate().child(line))
                    .child(
                        div()
                            .h(px(4.))
                            .rounded(crate::tokens::r(crate::tokens::R_BAR))
                            .bg(c(palette().line))
                            .child(
                                div()
                                    .h(px(4.))
                                    .rounded(crate::tokens::r(crate::tokens::R_BAR))
                                    .bg(c(palette().text))
                                    .w(relative(f.clamp(0.02, 1.))),
                            ),
                    ),
            );
        }
        let notice = self.notice()?;
        let (text, color) = match &notice {
            Ok(t) => (t, palette().text),
            Err(t) => (t, palette().warning),
        };
        let summary = text.lines().next().unwrap_or_default().to_owned();
        let details = (text != &summary || summary.chars().count() > 60).then(|| {
            crate::frame::button("alt-mods-details", tr("Details…")).on_click(cx.listener(
                move |this, _, _, cx| {
                    match &notice {
                        Ok(text) => this.open_details(text.clone()),
                        Err(text) => this.open_notice(text.clone(), None),
                    }
                    cx.notify();
                },
            ))
        });
        Some(
            div()
                .flex_none()
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
                        .line_clamp(2)
                        .child(summary),
                )
                .children(details)
                .child(
                    muted(tr("Dismiss"))
                        .id("alt-mods-dismiss")
                        .flex_none()
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
    fn mods_header(
        &mut self,
        installed: bool,
        focus: &FocusHandle,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        use ui::{c, field_theme};
        let t = field_theme(self.theme);
        let focused = focus.is_focused(window);
        let packs = matches!(self.ui.page, Page::Packs | Page::PackFind);
        let placeholder: SharedString = match (packs, installed) {
            (false, false) => tr("Search mods"),
            (false, true) => tr("Search your mods"),
            (true, false) => match self.ui.mods.packs.scope {
                pack_discover::Scope::Resource => tr("Search resource packs"),
                pack_discover::Scope::Shader => tr("Search shader packs"),
                pack_discover::Scope::Data => tr("Search data packs"),
            },
            (true, true) => match self.ui.mods.shelf {
                Shelf::Resource => tr("Search your resource packs"),
                Shelf::Shader => tr("Search your shader packs"),
                Shelf::Data => tr("Search your data packs"),
            },
        }
        .into();
        let edit = if installed {
            &self.ui.mods.have
        } else {
            &self.ui.mods.find
        };
        let narrow = self.content_width() < 680.;
        let field = crate::frame::input(focused)
            .id(if installed {
                "alt-mods-have"
            } else {
                "alt-mods-find"
            })
            .track_focus(focus)
            .seen()
            .min_w_0()
            .when(narrow, |d| d.w_full().flex_none().h(px(36.)))
            .when(!narrow, |d| d.flex_1().h(px(tokens::ACTION)))
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
                let m = &mut this.ui.mods;
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
                        }
                    }
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let field = if installed {
            line_edit::mouse(field, cx, |this: &mut Launcher| &mut this.ui.mods.have)
        } else {
            line_edit::mouse(field, cx, |this: &mut Launcher| &mut this.ui.mods.find)
        };
        let head = div()
            .flex()
            .flex_wrap()
            .items_center()
            .min_w_0()
            .gap(px(if narrow { 12. } else { 20. }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .child(
                        div()
                            .text_size(px(tokens::TITLE))
                            .font_weight(FontWeight::BOLD)
                            .child(if packs { tr("Packs") } else { tr("Mods") }),
                    )
                    .child(
                        div()
                            .id("mods-packs-section")
                            .reach()
                            .text_size(px(tokens::SMALL))
                            .text_color(c(palette().accent_ink))
                            .cursor_pointer()
                            .child(if packs {
                                tr("Back to mods")
                            } else {
                                tr("Resource, shader and data packs")
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if packs {
                                    this.open_mods()
                                } else {
                                    this.open_packs()
                                }
                                cx.notify();
                            })),
                    ),
            )
            .child(segmented(
                "alt-mods-tabs",
                [
                    (
                        "alt-mods-tab-discover",
                        tr("Discover").into(),
                        !installed,
                        if packs {
                            Page::PackFind
                        } else {
                            Page::Discover
                        },
                    ),
                    (
                        "alt-mods-tab-installed",
                        tr("Installed").into(),
                        installed,
                        if packs { Page::Packs } else { Page::Installed },
                    ),
                ],
                Launcher::choose_view,
                cx,
            ));
        if narrow {
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap(px(8.))
                .child(head)
                .child(field)
        } else {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(20.))
                .child(head)
                .child(field)
        }
    }
}
