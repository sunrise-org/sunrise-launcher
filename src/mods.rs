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
use std::path::{Path, PathBuf};
use sunrise_launcher::browser::{
    installed::{self, Entry},
    mod_list, sunrise, Source, SourceId,
};

mod actions;
mod build_modal;
pub(crate) mod build_run;
mod collection;
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
    /// (mod id, project id, title)
    Remove(Vec<(String, String, String)>),
    Update(model::UpdatePlan),
}

#[derive(PartialEq, Eq)]
struct OpenScope {
    id: String,
    home: PathBuf,
    target: Option<String>,
}

struct Open {
    id: String,
    request: u64,
    scope: Option<OpenScope>,
    /// The screen Back returns to.
    from: Page,
    page: Option<Result<Box<model::Page>, String>>,
    /// The collection's page this mod's was opened from: Back returns to it.
    parent: Option<String>,
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

#[derive(Default)]
struct InstalledRows {
    entries: Vec<Entry>,
    enabled: HashSet<String>,
    chosen: std::collections::BTreeMap<String, String>,
    local: Vec<model::Local>,
    switch_held: std::collections::HashMap<String, sunrise_launcher::managed_play::HeldReason>,
}

#[derive(Debug)]
enum InstalledReadError {
    GameInstall(installed::GameInstallProblem),
    Other(String),
}

type InstalledRead = Result<Option<(InstalledRows, Option<String>)>, InstalledReadError>;

impl InstalledRows {
    fn read(home: &Path) -> InstalledRead {
        use sunrise_launcher::managed_play::{try_display_active, DisplayActiveError};

        // Resolve the hard game journal before the advisory or the visible rows.
        let _lock = sunrise_launcher::browser::lock_home(home)
            .map_err(|error| InstalledReadError::Other(error.to_string()))?;
        installed::recover_game_install(home).map_err(InstalledReadError::GameInstall)?;
        let backup_notice = sunrise::auto::recover(home).err();
        let entries = installed::Installed::read(home)
            .map_err(InstalledReadError::Other)?
            .entries;
        let local = read_local(home);
        let enabled = enabled_ids(home, &entries, &local);
        let chosen = sunrise::chosen(home);
        let switch_held = match try_display_active(home) {
            Ok(active) => active
                .iter()
                .flat_map(|active| active.holds())
                .map(|hold| (hold.id().to_owned(), hold.reason().clone()))
                .collect(),
            Err(DisplayActiveError::Publishing) => return Ok(None),
            Err(DisplayActiveError::Read(error)) => return Err(InstalledReadError::Other(error)),
        };
        Ok(Some((
            Self {
                entries,
                enabled,
                chosen,
                local,
                switch_held,
            },
            backup_notice,
        )))
    }
}

struct Snapshot {
    key: String,
    home: PathBuf,
    rows: InstalledRows,
    reading: bool,
    error: Option<InstalledReadError>,
    backup_notice: Option<String>,
}

impl std::ops::Deref for Snapshot {
    type Target = InstalledRows;

    fn deref(&self) -> &Self::Target {
        &self.rows
    }
}

impl Snapshot {
    fn finish(&mut self, result: InstalledRead) -> bool {
        self.reading = matches!(result, Ok(None));
        self.error = match result {
            Ok(Some((rows, backup_notice))) => {
                self.rows = rows;
                self.backup_notice = backup_notice;
                None
            }
            Ok(None) => {
                self.key.clear(); // Publication was busy; ask again without publishing guessed rows.
                None
            }
            Err(error) => Some(error),
        };
        self.ready()
    }

    fn ready(&self) -> bool {
        !self.reading && self.error.is_none()
    }
}

enum InstalledRefresh<'a> {
    Reading,
    Failed(&'a InstalledReadError, &'a Path),
    Ready,
}

impl InstalledRefresh<'_> {
    fn of(snapshot: Option<&Snapshot>) -> InstalledRefresh<'_> {
        match snapshot {
            Some(snapshot) if !snapshot.reading => match &snapshot.error {
                Some(error) => InstalledRefresh::Failed(error, &snapshot.home),
                None => InstalledRefresh::Ready,
            },
            _ => InstalledRefresh::Reading,
        }
    }
}

enum Event {
    /// Installed rows read for the requested folder and fingerprint, off the window thread.
    Local(String, InstalledRead),
    Enabled(
        PathBuf,
        actions::EnableNotice,
        Result<actions::EnabledMods, String>,
        Option<u64>,
    ),
    Catalog(String, Result<(Vec<Card>, Vec<String>), String>),
    Page(u64, Result<Box<model::Page>, String>),
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
    catalog_game: Option<String>,
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
    page_request: u64,
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

impl ModsState {
    fn visible_notice(
        &self,
        shared: Option<&Result<String, String>>,
    ) -> Option<Result<String, String>> {
        self.notice
            .as_ref()
            .or(shared)
            .cloned()
            .or_else(|| self.snapshot.as_ref()?.backup_notice.clone().map(Ok))
    }
}

impl Default for ModsState {
    fn default() -> Self {
        let (tx, rx) = crate::wake::channel();
        Self {
            tx,
            rx,
            catalog: Catalog::Idle,
            catalog_game: None,
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
            page_request: 0,
            picks: None,
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

/// The mods placed by hand in `home`: every package is opened and read.
fn read_local(home: &Path) -> Vec<model::Local> {
    let local = |native: bool| {
        move |f: native_status::Installed| model::Local {
            id: f.id,
            name: f.name,
            version: f.version,
            github: f.origin.is_some(),
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
            Page::Project => {
                let scope = self.mods_page_scope();
                if self
                    .ui
                    .mods
                    .open
                    .as_ref()
                    .is_some_and(|open| open.scope != scope)
                {
                    let open = self.ui.mods.open.take().expect("different open scope");
                    self.ui.mods.picks = None;
                    self.mods_load_page(open.id, open.from);
                    if let Some(now) = self.ui.mods.open.as_mut() {
                        now.parent = open.parent;
                    }
                }
                self.project(cx)
            }
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
            .then(|| self.ui.mods.open.as_ref().map(|o| o.id.clone()))
            .flatten()
            .filter(|open| self.is_collection(open));
        self.ui.page = Page::Project;
        self.mods_load_page(id, from);
        if let Some(open) = self.ui.mods.open.as_mut() {
            open.parent = parent;
        }
    }

    fn is_collection(&self, id: &str) -> bool {
        matches!(&self.ui.mods.catalog, Catalog::Ready(cards, _)
            if cards.iter().any(|c| c.id == id && c.members.is_some()))
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

    pub(crate) fn mods_home(&self) -> PathBuf {
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
                Event::Catalog(game, result) => {
                    if m.catalog_game.as_ref() == Some(&game) {
                        m.catalog = match result {
                            Ok((cards, categories)) => Catalog::Ready(cards, categories),
                            Err(error) => Catalog::Failed(error),
                        };
                    }
                }
                Event::Local(key, result) => {
                    if let Some(snap) = m.snapshot.as_mut().filter(|s| s.key == key) {
                        if snap.finish(result) {
                            let rows: HashSet<String> =
                                self.rows().into_iter().map(|r| r.id).collect();
                            self.ui.mods.selection.retain(|id| rows.contains(id));
                        }
                    }
                }
                Event::Enabled(home, notice, result, allowed_at) => {
                    self.enabled_mods(&home, notice, result, allowed_at);
                }
                Event::Page(request, page) => {
                    if let Some(open) = m.open.as_mut().filter(|o| o.request == request) {
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
                    m.notice = Some(match failed.is_empty() {
                        true => Ok(said),
                        false => Err(format!(
                            "{said} {}",
                            trf(
                                "Not done: {list}",
                                &[(
                                    "list",
                                    &failed
                                        .iter()
                                        .map(|(id, why)| format!("{id}: {why}"))
                                        .collect::<Vec<_>>()
                                        .join("; ")
                                )]
                            )
                        )),
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
            // Keep the same profile's confirmed rows and advisory until the worker answers.
            let (rows, backup_notice) = self
                .ui
                .mods
                .snapshot
                .take()
                .filter(|s| s.home == home)
                .map(|s| (s.rows, s.backup_notice))
                .unwrap_or_default();
            let (tx, worker_home, worker_key) =
                (self.ui.mods.tx.clone(), home.clone(), key.clone());
            std::thread::spawn(move || {
                let result = InstalledRows::read(&worker_home);
                if matches!(result, Ok(None)) {
                    // Delay just the busy display retry, without holding any profile/cache lock.
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                let _ = tx.send(Event::Local(worker_key, result));
            });
            self.ui.mods.snapshot = Some(Snapshot {
                key,
                home,
                rows,
                reading: true,
                error: None,
                backup_notice,
            });
        }
        let wants_index = self
            .ui
            .mods
            .snapshot
            .as_ref()
            .is_some_and(|s| !s.rows.entries.is_empty())
            || self.ui.page != Page::Installed;
        if wants_index {
            self.ensure_mod_catalog();
        }
    }

    pub(crate) fn ensure_mod_catalog(&mut self) {
        let game = self.mods_game();
        if self.ui.mods.catalog_game.as_ref() == Some(&game)
            && !matches!(self.ui.mods.catalog, Catalog::Idle)
        {
            return;
        }
        self.ui.mods.catalog_game = Some(game.clone());
        self.ui.mods.catalog = Catalog::Loading;
        let tx = self.ui.mods.tx.clone();
        std::thread::spawn(move || {
            let result = sunrise::index().map(|index| {
                let cards = model::cards(&index, &game);
                let categories = model::categories(&cards);
                (cards, categories)
            });
            let _ = tx.send(Event::Catalog(game, result));
        });
    }

    fn mods_page_scope(&self) -> Option<OpenScope> {
        let instance = self.selected_instance()?;
        Some(OpenScope {
            id: instance.id.clone(),
            home: instance.home.clone(),
            target: self.profile_target(instance),
        })
    }

    fn mods_load_page(&mut self, id: String, from: Page) {
        let scope = self.mods_page_scope();
        self.ui.mods.page_request += 1;
        let request = self.ui.mods.page_request;
        self.ui.mods.open = Some(Open {
            id: id.clone(),
            request,
            scope,
            from,
            page: None,
            parent: None,
        });
        // A collection's page reads the index when it draws.
        if sunrise::known_index().is_some_and(|index| index.collection(&id).is_some()) {
            return;
        }
        let (tx, game) = (self.ui.mods.tx.clone(), self.mods_game());
        let instance = self.selected_instance().cloned();
        std::thread::spawn(move || {
            let result = sunrise::index().and_then(|index| {
                let trust = sunrise::trust_and_capabilities(&id, instance.as_ref(), &game);
                let another = instance
                    .as_ref()
                    .is_some_and(|instance| sunrise::made_for_another_build(&id, instance, &game));
                model::page(&index, &id, &game, trust, another)
                    .map(Box::new)
                    .ok_or_else(|| trf("{id} isn't a mod in Sunrise's registry.", &[("id", &id)]))
            });
            let _ = tx.send(Event::Page(request, result));
        });
    }

    pub(crate) fn registry_install_hold(&self, id: &str) -> Option<installed::InstallHold> {
        let snap = self
            .ui
            .mods
            .snapshot
            .as_ref()
            .filter(|snap| snap.home == self.mods_home())?;
        if snap.enabled.contains(id) {
            return None;
        }
        snap.entries
            .iter()
            .find(|entry| entry.source == SourceId::Sunrise && entry.project == id)?
            .install_hold
            .clone()
    }

    /// The installed mods as rows. Uses only the index served over HTTPS this run for registry
    /// names and statuses. Never asks the network or writes.
    fn rows(&self) -> Vec<Row> {
        let Some(snap) = &self.ui.mods.snapshot else {
            return Vec::new();
        };
        let unbuilt: HashSet<String> = match !snap.rows.local.is_empty()
            || snap
                .rows
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
        let index = sunrise::fetched_index();
        let game = self.mods_game();
        let mut rows = model::rows(
            &snap.rows.entries,
            &model::World {
                index: index.as_deref(),
                game: &game,
                enabled: &snap.rows.enabled,
                unbuilt: &unbuilt,
                native_allowed: self.settings.native_mods_allowed_at.is_some(),
                chosen: &snap.rows.chosen,
                local: &snap.rows.local,
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
        rows
    }

    /// Whether another install or update holds the folder right now.
    fn mods_busy(&self) -> bool {
        self.mutating()
    }

    /// Whether a mod change is in progress (mine or the shared install's), for the shared Play
    /// and setup gates. Reads only the Mods busy flags, never the launch state.
    pub(super) fn mutating(&self) -> bool {
        self.ui.mods.busy.is_some() || self.registry_stage().is_some()
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

    /// A backup advisory should never eclipse either operation's own result.
    fn notice(&self) -> Option<Result<String, String>> {
        self.ui.mods.visible_notice(self.mods.message.as_ref())
    }

    fn dismiss_notice(&mut self) {
        if self.ui.mods.notice.is_none() && self.mods.message.is_none() {
            if let Some(snapshot) = &mut self.ui.mods.snapshot {
                snapshot.backup_notice = None;
            }
        }
        self.ui.mods.notice = None;
        self.mods.message = None;
    }

    /// The strip shown above either list: progress, or the last result.
    fn strip(&self, cx: &mut Context<Self>) -> Option<Div> {
        use ui::{c, muted};
        // Registry installs use the global strip and the clicked card, not a third bar here.
        if let Some((line, f)) = self.ui.mods.busy.clone() {
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
            native: false,
            unreadable: false,
        };
        // "gone" was a package of the list known before and is no longer there.
        let got = enabled_ids(home.path(), &[], &[local("kept"), local("off")]);
        assert_eq!(got, HashSet::from(["kept".to_string()]));
    }
}

#[cfg(test)]
mod installed_refresh_tests {
    use super::*;

    #[test]
    fn busy_publication_keeps_the_settled_hold_and_disables_actions_until_refresh() {
        use sunrise_launcher_smod::{
            locks::{LockMode, Release},
            runtime_leases::Registry,
        };
        const ACTOR: &str = "INSTALLED_BUSY_PUBLICATION_ACTOR";
        let Some(root) = std::env::var_os(ACTOR).map(PathBuf::from) else {
            let scratch = tempfile::tempdir().unwrap();
            let root = scratch.path();
            assert!(std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "mods::installed_refresh_tests::busy_publication_keeps_the_settled_hold_and_disables_actions_until_refresh"])
                .env(ACTOR, root).env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
                .status().unwrap().success());
            return;
        };
        let home = root.join("game");
        let registry = Registry::with_release(&backend::data_dir(), Release::Close).unwrap();
        let publication = registry.publication(LockMode::Exclusive).unwrap();
        let mut snapshot = Snapshot {
            key: "viewed fingerprint".into(),
            home: home.clone(),
            rows: InstalledRows {
                switch_held: std::collections::HashMap::from([(
                    "ruby".into(),
                    sunrise_launcher::managed_play::HeldReason::NoFit {
                        build: "fixture".into(),
                    },
                )]),
                ..Default::default()
            },
            reading: true,
            error: None,
            backup_notice: None,
        };
        assert!(!snapshot.finish(InstalledRows::read(&home)));
        assert!(!snapshot.ready());
        assert!(matches!(
            InstalledRefresh::of(Some(&snapshot)),
            InstalledRefresh::Reading
        ));
        assert!(snapshot.switch_held.contains_key("ruby"));
        assert!(
            snapshot.key.is_empty(),
            "the same fingerprint must be reread"
        );
        drop(publication);
        assert!(snapshot.finish(InstalledRows::read(&home)));
        assert!(snapshot.ready());
    }

    #[test]
    fn retry_after_a_game_journal_error_keeps_the_last_good_rows_until_success() {
        let home = tempfile::tempdir().unwrap();
        let journal = home
            .path()
            .join(sunrise_launcher_smod::index::browser_state::PENDING);
        std::fs::write(&journal, b"not a journal").unwrap();
        let mut snapshot = Snapshot {
            key: String::new(),
            home: home.path().into(),
            rows: InstalledRows {
                enabled: HashSet::from(["ruby".into()]),
                ..Default::default()
            },
            reading: true,
            error: None,
            backup_notice: None,
        };
        assert!(matches!(
            InstalledRefresh::of(None),
            InstalledRefresh::Reading
        ));
        assert!(matches!(
            InstalledRefresh::of(Some(&snapshot)),
            InstalledRefresh::Reading
        ));
        snapshot.finish(InstalledRows::read(home.path()));
        assert!(snapshot.enabled.contains("ruby"));
        assert!(matches!(
            InstalledRefresh::of(Some(&snapshot)),
            InstalledRefresh::Failed(InstalledReadError::GameInstall(_), _)
        ));
        assert_eq!(std::fs::read(&journal).unwrap(), b"not a journal");
        std::fs::remove_file(journal).unwrap();
        snapshot.finish(InstalledRows::read(home.path()));
        assert!(matches!(
            InstalledRefresh::of(Some(&snapshot)),
            InstalledRefresh::Ready
        ));
        assert!(snapshot.enabled.is_empty());
    }

    #[test]
    fn the_backup_advisory_is_the_last_fallback() {
        let backup = "backup kept".to_owned();
        let mut mods = ModsState {
            snapshot: Some(Snapshot {
                key: String::new(),
                home: PathBuf::new(),
                rows: InstalledRows::default(),
                reading: false,
                error: None,
                backup_notice: Some(backup.clone()),
            }),
            ..ModsState::default()
        };
        let install = Err("shared install error".to_owned());
        assert_eq!(mods.visible_notice(Some(&install)), Some(install.clone()));
        let batch = Err("batch error".to_owned());
        mods.notice = Some(batch.clone());
        assert_eq!(mods.visible_notice(Some(&install)), Some(batch));
        mods.notice = None;
        assert_eq!(mods.visible_notice(None), Some(Ok(backup)));
    }
}
