//! Mods: Discover, Installed and a mod's page.
//!
//! **Real:** the registry list, search, categories, fit, kind and sort (`browser::sunrise`'s index,
//! read in a worker); a mod's page from the index (versions and their fit, size, licence,
//! authors, source, mirrored screenshots, what it may do); Add and single Update through the
//! existing install path (native code asks for permission first, `native_consent`); Installed
//! from `mod-browser.json` and `sunrise-mods.txt` with on/off, selection, filters, bulk Turn on,
//! Turn off, Remove and Update N mods (a reviewed plan, applied by a worker); Build…
//! hands over to the one native build path.
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
use std::path::{Path, PathBuf};
use sunrise_launcher::browser::{
    installed::{self, Entry},
    mod_list, sunrise, SourceId,
};

mod actions;
mod build_modal;
pub(crate) mod build_run;
mod collection;
mod data_packs;
mod discover;
pub(crate) mod install_state;
mod installed_view;
mod model;
mod pack_discover;
mod pack_project;
mod packs;
mod project;
mod search;
mod ui;
pub(crate) use actions::Lock;
pub(crate) use data_packs::names;
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
    /// (mod id, exact installed row, title)
    Remove(Vec<(String, Entry, String)>),
}

#[derive(Debug, PartialEq, Eq)]
struct DetailContext {
    home: PathBuf,
    build: Option<String>,
    #[allow(clippy::type_complexity)]
    active: Option<(u64, Option<std::time::SystemTime>)>,
}

struct Open {
    id: String,
    request: u64,
    /// The screen Back returns to.
    from: Page,
    page: Option<Result<Box<model::Page>, String>>,
    collection: bool,
    /// The collection's page this mod's was opened from: Back returns to it.
    parent: Option<String>,
}

impl Open {
    fn finish(&mut self, request: u64, page: Result<Box<model::Page>, String>) {
        if self.request == request && self.page.is_none() {
            self.page = Some(page);
        }
    }
}

/// The ids switched on among the registry entries and the hand-placed packages.
fn enabled_ids(home: &Path, entries: &[Entry], local: &[model::Local]) -> HashSet<String> {
    let mut enabled: HashSet<String> = entries
        .iter()
        .filter_map(|e| sunrise::mod_id(e))
        .filter(|id| mod_list::is_enabled(home, id))
        .map(str::to_owned)
        .collect();
    // A file that can't be read has no id of its own (its file name is only a label), so it is
    // never on: it must not borrow the on state of a working mod of that name.
    enabled.extend(
        local
            .iter()
            .filter(|l| !l.unreadable && mod_list::is_enabled(home, &l.id))
            .map(|l| l.id.clone()),
    );
    enabled
}

struct Snapshot {
    key: String,
    home: PathBuf,
    entries: Vec<Entry>,
    enabled: HashSet<String>,
    chosen: std::collections::BTreeMap<String, String>,
    waiting: std::collections::BTreeMap<String, sunrise::txn::WaitingMod>,
    /// Mod files the registry didn't install: on/off is editable, the files are not.
    local: Vec<model::Local>,
    /// The hand-placed packages are being read off the window thread (`Event::Local`): `local`
    /// holds what was known before.
    reading: bool,
}

enum Event {
    /// The hand-placed mods read for the folder and fingerprint the snapshot was made for.
    Local(
        String,
        Vec<model::Local>,
        std::collections::BTreeMap<String, sunrise::txn::WaitingMod>,
    ),
    Catalog(Result<(Vec<Card>, Vec<String>), String>),
    Page(u64, DetailContext, Result<Box<model::Page>, String>),
    Progress(String, f32),
    /// A pack search's answer, by the number of the search that asked.
    Packs(u64, Result<Vec<sunrise_launcher::browser::Project>, String>),
    /// A pack's page, by its source and id.
    PackPage((SourceId, String), pack_project::PackPage),
    /// A resource or data pack install ended: what it said.
    PackDone(Result<String, String>),
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
    /// The row last clicked, for Shift-click ranges (an index into the visible rows).
    anchor: Option<usize>,
    confirm: Option<Confirm>,
    /// The folder the open confirmation was made for.
    reviewed_home: Option<PathBuf>,
    snapshot: Option<Snapshot>,
    open: Option<Open>,
    next_detail: u64,
    /// The collection whose page is open and the members ticked on it.
    picks: Option<collection::Picks>,
    /// My own batch (update or remove): its line and fraction.
    busy: Option<(String, f32)>,
    notice: Option<Result<String, String>>,
    /// Why a mod's last update failed, shown on its row until the next attempt.
    errors: HashMap<String, String>,
    /// How many cards Discover draws ("Show more" adds a page).
    limit: usize,
    /// The Installed row whose "?" is open (a mod not on this build), and whether its
    /// technical details show.
    /// The build the build window shows: running, or ended and not yet closed.
    pub(crate) build: Option<build_run::Run>,
    /// Build was pressed and the build has not started yet (it asks for permission first).
    pub(crate) build_wanted: bool,
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
            confirm: None,
            reviewed_home: None,
            snapshot: None,
            open: None,
            next_detail: 0,
            picks: None,
            busy: None,
            notice: None,
            errors: HashMap::new(),
            limit: 0,
            build: None,
            build_wanted: false,
            facts_open: false,
            last_mods: None,
            last_packs: None,
            reveal: None,
            revealed: Default::default(),
            viewer: None,
        }
    }
}

/// The mods placed by hand in `home`: every package is opened and read.
fn read_local(home: &Path) -> Vec<model::Local> {
    let local = |native: bool| {
        move |f: native_status::Installed| model::Local {
            id: f.id,
            name: f.name,
            version: f.version,
            github: f.origin.is_some(),
            source: f
                .origin
                .and_then(|o| model::SourceLink::read(&o.source_url())),
            native,
            unreadable: f.icon.is_err(),
        }
    };
    native_status::untracked(home)
        .into_iter()
        .map(local(true))
        .chain(
            native_status::untracked_content(home)
                .into_iter()
                .map(local(false)),
        )
        .collect()
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
            Page::Installed => Page::Installed,
            _ => Page::Discover,
        };
        // A collection's page opened from a list starts from its defaults again.
        if self.is_collection(&id) && self.ui.page != Page::Project {
            self.ui.mods.picks = None;
        }
        let parent = (self.ui.page == Page::Project)
            .then(|| {
                self.ui
                    .mods
                    .open
                    .as_ref()
                    .filter(|o| o.collection)
                    .map(|o| o.id.clone())
            })
            .flatten();
        self.ui.page = Page::Project;
        self.mods_load_page(id, from, None);
        if let Some(open) = self.ui.mods.open.as_mut() {
            open.parent = parent;
        }
    }

    fn mods_show_installed_project(&mut self, row: Row) {
        let id = row.entry.as_ref().map_or(&row.id, |e| &e.project).clone();
        self.ui.page = Page::Project;
        self.mods_load_page(id, Page::Installed, Some(row));
    }

    fn is_collection(&self, id: &str) -> bool {
        matches!(&self.ui.mods.catalog, Catalog::Ready(cards, _)
            if cards.iter().any(|c| c.id == id && c.members.is_some()))
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

    /// The Mods tab: where the player last was this session, else Installed when this setup
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

    /// The Packs tab, by the same rule as [`Self::open_mods`].
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
                Event::Local(key, local, waiting) => {
                    if let Some(snap) = m.snapshot.as_mut().filter(|s| s.key == key) {
                        snap.enabled = enabled_ids(&snap.home, &snap.entries, &local);
                        snap.local = local;
                        snap.waiting = waiting;
                        snap.reading = false;
                    }
                    let rows: HashSet<String> = self.rows().into_iter().map(|r| r.id).collect();
                    self.ui.mods.selection.retain(|id| rows.contains(id));
                }
                Event::Page(request, context, page) => {
                    if self.detail_context() == context {
                        if let Some(open) = &mut self.ui.mods.open {
                            open.finish(request, page);
                        }
                    }
                }
                Event::Progress(line, f) => m.busy = Some((line, f)),
                Event::Packs(seq, result) => self.pack_answer(seq, result),
                Event::PackPage(key, page) => self.pack_page_answer(key, page),
                Event::PackDone(result) => self.pack_installed(result),
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
        let key = format!(
            "{}\n{:?}",
            native_status::fingerprint(&home, None),
            sunrise::txn::active_stamp(&home)
        );
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
            // Reading every hand-placed package takes long with many of them: it is done on a
            // worker, and the rows of the same folder known before stay until it answers.
            let (local, waiting) = self
                .ui
                .mods
                .snapshot
                .take()
                .filter(|s| s.home == home)
                .map(|s| (s.local, s.waiting))
                .unwrap_or_default();
            let (tx, worker_home, worker_key) =
                (self.ui.mods.tx.clone(), home.clone(), key.clone());
            std::thread::spawn(move || {
                let _ = tx.send(Event::Local(
                    worker_key,
                    read_local(&worker_home),
                    sunrise::txn::waiting_for_update(&worker_home),
                ));
            });
            let enabled = enabled_ids(&home, &entries, &local);
            let chosen = sunrise::chosen(&home);
            self.ui.mods.snapshot = Some(Snapshot {
                key,
                home,
                entries,
                enabled,
                chosen,
                waiting,
                local,
                reading: true,
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

    fn detail_context(&self) -> DetailContext {
        let home = self.mods_home();
        DetailContext {
            build: self.selected().cloned(),
            active: sunrise::txn::active_stamp(&home),
            home,
        }
    }

    fn mods_load_page(&mut self, id: String, from: Page, installed: Option<Row>) {
        self.ui.mods.facts_open = false;
        let collection = installed.is_none() && self.is_collection(&id);
        let (page, installed) = match installed {
            Some(row) if row.entry.is_none() => (Some(Ok(Box::new(row.installed_details()))), None),
            other => (None, other),
        };
        // Collections read the index when drawn; local files already have all their details.
        let load = !collection && page.is_none();
        self.ui.mods.next_detail += 1;
        let request = self.ui.mods.next_detail;
        self.ui.mods.open = Some(Open {
            id: id.clone(),
            request,
            from,
            page,
            collection,
            parent: None,
        });
        if !load {
            return;
        }
        let (tx, game) = (self.ui.mods.tx.clone(), self.mods_game());
        let instance = self
            .selected_instance()
            .cloned()
            .unwrap_or_else(instances::selected);
        let context = self.detail_context();
        std::thread::spawn(move || {
            let result = sunrise::index().and_then(|index| {
                let trust = sunrise::trust_and_capabilities(&id, Some(&instance), &game);
                let another = sunrise::made_for_another_build(&id, &instance, &game);
                model::page(&index, &id, &game, trust, another)
                    .map(Box::new)
                    .ok_or_else(|| trf("{id} isn't a mod in Sunrise's registry.", &[("id", &id)]))
            });
            let result = match (result, installed) {
                (Err(_), Some(row)) => Ok(Box::new(row.installed_details())),
                (result, _) => result,
            };
            let _ = tx.send(Event::Page(request, context, result));
        });
    }

    /// The installed mods as rows. Uses only the index served over HTTPS this run for registry
    /// names and statuses. Never asks the network or writes.
    fn rows(&self) -> Vec<Row> {
        let Some(snap) = &self.ui.mods.snapshot else {
            return Vec::new();
        };
        let index = sunrise::fetched_index();
        let game = self.mods_game();
        model::rows(
            &snap.entries,
            &model::World {
                index: index.as_deref(),
                game: &game,
                enabled: &snap.enabled,
                chosen: &snap.chosen,
                waiting: &snap.waiting,
                local: &snap.local,
            },
        )
    }

    /// Whether another install or update holds the folder right now.
    fn mods_busy(&self) -> bool {
        self.mutating()
    }

    /// Whether a mod change is in progress (mine or the shared install's), for the shared Play
    /// and setup gates. Reads only the Mods busy flags, never the launch state.
    pub(super) fn mutating(&self) -> bool {
        self.ui.mods.busy.is_some() || self.mods.busy.is_some() || self.mods.world_ask.is_some()
    }

    /// Build… on a row: the one native build path, which asks for permission first. The player
    /// stays where they are: the play bar shows how far it is.
    fn build(&mut self) {
        self.build_native();
    }

    /// Open a browser destination already checked by its caller.
    pub(super) fn open_url(url: &str, cx: &mut Context<Self>) {
        cx.open_url(url);
    }

    /// What the notice strip says: the batch's own, else what the shared install path said.
    fn notice(&self) -> Option<Result<String, String>> {
        self.ui
            .mods
            .notice
            .clone()
            .or_else(|| self.mods.message.clone())
    }

    fn dismiss_notice(&mut self) {
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
                    .child(div().text_size(px(tokens::LABEL)).child(line))
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
        let (text, color) = match notice {
            Ok(t) => (t, palette().text),
            Err(t) => (t, palette().warning),
        };
        Some(
            div()
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
                        .child(text),
                )
                .child(
                    muted(tr("Dismiss"))
                        .id("alt-mods-dismiss")
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
        use ui::field_theme;
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
        let field = crate::frame::input(focused)
            .id(if installed {
                "alt-mods-have"
            } else {
                "alt-mods-find"
            })
            .track_focus(focus)
            .seen()
            .flex_1()
            .min_w(px(120.))
            .h(px(tokens::ACTION))
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
        div()
            .flex()
            .items_center()
            .gap(px(20.))
            .child(
                div()
                    .flex_none()
                    .text_size(px(tokens::TITLE))
                    .font_weight(FontWeight::BOLD)
                    .child(if packs { tr("Packs") } else { tr("Mods") }),
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
            ))
            .child(field)
    }
}

#[cfg(test)]
mod enabled_tests {
    use super::*;

    #[test]
    fn reopening_the_same_mod_discards_the_old_build_fit() {
        let page = |fit| {
            Ok(Box::new(model::Page {
                id: "ruby".into(),
                name: "Ruby".into(),
                authors: vec![],
                description: String::new(),
                source: None,
                about: None,
                categories: vec![],
                screenshots: vec![],
                icon: None,
                make: None,
                installed: None,
                versions: vec![],
                fit,
                will_install: None,
                trust: vec![],
                another_build: false,
                requires: vec![],
            }))
        };
        let mut open = Open {
            id: "ruby".into(),
            request: 2,
            from: Page::Installed,
            page: None,
            collection: false,
            parent: None,
        };
        open.finish(1, page(model::Fit::OtherBuild("26.2".into())));
        assert!(open.page.is_none());
        open.finish(2, page(model::Fit::Fits));
        open.finish(1, page(model::Fit::OtherBuild("26.2".into())));
        assert_eq!(open.page.unwrap().unwrap().fit, model::Fit::Fits);
    }

    #[test]
    fn enabled_ids_come_from_the_current_packages_only() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(
            home.path().join("sunrise-mods.txt"),
            "mod.kept: true\nmod.gone: true\nmod.off: false\n",
        )
        .unwrap();
        let local = |id: &str| model::Local {
            id: id.into(),
            name: id.into(),
            version: String::new(),
            github: false,
            source: None,
            native: false,
            unreadable: false,
        };
        // "gone" was a package of the list known before and is no longer there.
        let got = enabled_ids(home.path(), &[], &[local("kept"), local("off")]);
        assert_eq!(got, HashSet::from(["kept".to_string()]));
    }
}
