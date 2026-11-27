//! Mods: Discover, Installed and a mod's page.
//!
//! **Real:** the registry list, search, categories, fit, kind and sort (`browser::sunrise`'s index,
//! read in a worker); a mod's page from the index (versions and their fit, size, licence,
//! authors, source, mirrored screenshots, what it may do); Add and single Update through the
//! existing install path; Installed from `mod-browser.json` and `sunrise-mods.txt` with on/off,
//! selection, filters, bulk Turn on, Turn off, Remove and Update N mods (a reviewed plan, applied
//! by a worker); Build… hands over to the one native build path.
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
    mod_list, sunrise, Kind, SourceId,
};

mod actions;
mod build_modal;
pub(crate) mod build_run;
mod collection_v2;
mod data_packs;
mod discover;
mod installed_view;
mod model;
mod nightly;
mod pack_discover;
mod pack_project;
mod packs;
mod ports;
pub(crate) use ports::PortSwitch;
mod project;
mod rating;
pub(crate) mod refused;
mod search;
mod ui;
pub(crate) use data_packs::names;
pub(crate) use search::Installed;
mod viewer;
use std::{rc::Rc, sync::Arc};
use sunrise_launcher::native_mods::local_fit;
pub(crate) use ui::{
    checkbox, choices, empty_state, field_theme, reveal, segmented, switch, Choose,
};

use crate::paint::palette;
use model::Row;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Menu {
    Category,
    Sort,
}

struct V2CatalogPayload {
    cards: Vec<model::discover_v2::V2Card>,
    collections: Vec<model::discover_v2::V2Collection>,
    categories: Vec<String>,
    /// The index lists the selected profile's build among its offered builds.
    build_listed: bool,
    stale_warning: Option<String>,
    from_cache: bool,
}

enum V2Catalog {
    Idle,
    Loading,
    Ready {
        cards: Rc<[model::discover_v2::V2Card]>,
        collections: Rc<[model::discover_v2::V2Collection]>,
        categories: Rc<[String]>,
        build_listed: bool,
        stale_warning: Option<String>,
        from_cache: bool,
    },
    Failed(String),
}

/// This computer's fit check of the releases the index says nothing of on one build.
#[derive(Default)]
struct LocalFitRun {
    /// The build checked last, or being checked.
    build: Option<String>,
    running: bool,
    /// Why the last check of `build` stopped; it isn't started again until Try again.
    failed: Option<String>,
}

#[derive(PartialEq, Eq)]
struct OpenScope {
    profile: Option<String>,
    home: PathBuf,
    target: Option<String>,
    game: String,
}

enum InstalledProject {
    Registry,
    Local(String),
}

struct Open {
    id: String,
    installed: Option<InstalledProject>,
    collection: bool,
    v2: bool,
    request: u64,
    scope: OpenScope,
    /// The screen Back returns to.
    from: Page,
    v2_page: Option<Result<Box<model::discover_v2::V2Page>, String>>,
    /// The collection's page this mod's was opened from: Back returns to it.
    parent: Option<String>,
    /// The installed mods this page's Remove asked to remove, recorded while their rows are
    /// still read: the page leaves once that Remove finishes.
    removing: Vec<String>,
}

impl Open {
    fn finish_v2(
        &mut self,
        request: u64,
        scope: &OpenScope,
        page: Result<Box<model::discover_v2::V2Page>, String>,
    ) {
        if self.request == request && &self.scope == scope && self.v2_page.is_none() {
            self.v2_page = Some(page);
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

struct ArchiveDisplay {
    name: String,
    authors: Vec<String>,
    version: String,
    icon: Option<std::sync::Arc<gpui::Image>>,
}

#[derive(Default)]
struct InstalledRows {
    entries: Vec<Entry>,
    archives: HashMap<String, ArchiveDisplay>,
    enabled: HashSet<String>,
    chosen: std::collections::BTreeMap<String, String>,
    local: Vec<model::Local>,
    managed: Option<Box<sunrise_launcher::managed_play::Active>>,
    unbuilt: HashSet<String>,
    switch_held: std::collections::HashMap<String, sunrise_launcher::managed_play::HeldReason>,
    /// Held local or source mods: their switch checks their exact bytes again.
    local_holds: HashSet<String>,
}

const ADDED_PENDING: &str = "Added · builds when you play";

#[derive(Debug)]
enum InstalledReadError {
    GameInstall(installed::GameInstallProblem),
    Other(String),
}

type InstalledRead = Result<Option<(InstalledRows, Option<String>)>, InstalledReadError>;

impl InstalledRows {
    fn read(home: &Path, profile: native_mods::Profile, base: Option<&str>) -> InstalledRead {
        use sunrise_launcher::managed_play::{try_display_active, DisplayActiveError};

        // Resolve the hard game journal before the advisory or the visible rows.
        let _lock = sunrise_launcher::browser::lock_home(home)
            .map_err(|error| InstalledReadError::Other(error.to_string()))?;
        installed::recover_game_install(home).map_err(InstalledReadError::GameInstall)?;
        let backup_notice = sunrise::auto::recover(home).err();
        let entries = installed::Installed::read(home)
            .map_err(InstalledReadError::Other)?
            .entries;
        // Display metadata only; catalogue status and action authority still use their own proofs.
        let mut archives: HashMap<_, _> = entries
            .iter()
            .filter(|entry| entry.source == SourceId::Sunrise && entry.kind == Kind::Mod)
            .filter_map(|entry| {
                let id = sunrise::mod_id(entry)?;
                let package =
                    sunrise_launcher_smod::manifest::Package::read(&entry.path(home)?).ok()?;
                let manifest = package.manifest;
                (manifest.id == id).then(|| {
                    (
                        entry.file.clone(),
                        ArchiveDisplay {
                            name: manifest.name,
                            authors: manifest.authors,
                            version: manifest.version,
                            icon: package.icon.as_ref().map(package_image),
                        },
                    )
                })
            })
            .collect();
        let local = read_local(home, &mut archives);
        let enabled = enabled_ids(home, &entries, &local);
        let chosen = sunrise::chosen(home);
        let active = match try_display_active(home) {
            Ok(active) => active,
            Err(DisplayActiveError::Publishing) => return Ok(None),
            Err(DisplayActiveError::Read(error)) => return Err(InstalledReadError::Other(error)),
        };
        let switch_held = active
            .iter()
            .flat_map(|active| active.holds())
            .map(|hold| (hold.id().to_owned(), hold.reason().clone()))
            .collect();
        let local_holds = active
            .iter()
            .flat_map(|active| active.holds())
            .filter(|hold| {
                !matches!(
                    hold.source(),
                    sunrise_launcher::managed_play::HeldSource::Registry { .. }
                )
            })
            .map(|hold| hold.id().to_owned())
            .collect();
        let unbuilt = native_status::pending_for(home, active.as_ref(), profile, base)
            .into_iter()
            .collect();
        Ok(Some((
            Self {
                entries,
                archives,
                enabled,
                chosen,
                local,
                managed: active.map(Box::new),
                unbuilt,
                switch_held,
                local_holds,
            },
            backup_notice,
        )))
    }
}

/// What a frame of Installed drew: its rows read afresh, or something the benchmark mustn't time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum InstalledDrawn {
    /// A read is under way: nothing, or the rows of the read before it.
    Reading,
    /// The read failed: its error, over any rows from before it.
    Failed,
    /// Read, and the profile has no mods to list.
    Empty,
    /// Read, and its rows drawn.
    Rows,
}

impl InstalledDrawn {
    fn of(reading: bool, failed: bool, rows: usize) -> Self {
        match (reading, failed, rows) {
            (true, _, _) => Self::Reading,
            (_, true, _) => Self::Failed,
            (_, _, 0) => Self::Empty,
            _ => Self::Rows,
        }
    }
}

/// The benchmark's view of Installed after a frame that drew it as `drawn`, with its list's
/// offset and how far it scrolls (`laid`) when that frame drew the list. Only rows read afresh
/// are timed; a read still under way is waited for.
fn installed_target(
    drawn: Option<InstalledDrawn>,
    laid: Option<(f32, f32)>,
) -> crate::perf::Target {
    use crate::perf::Target;
    match (drawn, laid) {
        (Some(InstalledDrawn::Failed), _) => Target::Failed,
        (Some(InstalledDrawn::Empty), _) => Target::Empty,
        (Some(InstalledDrawn::Rows), Some((offset, max))) => Target::Laid { offset, max },
        _ => Target::LayingOut,
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
    Local(String, Box<InstalledRead>),
    Enabled(
        PathBuf,
        actions::EnableNotice,
        Result<actions::EnabledMods, String>,
    ),
    V2Catalog(
        Option<String>,
        Result<V2CatalogPayload, sunrise::v2::LoadError>,
    ),
    /// The verified snapshot on disk was read, ahead of the network read: redraw from it.
    V2Saved,
    /// This computer's fit check of a build finished.
    LocalFit(String, Result<(), String>),
    V2Page(u64, Result<Box<model::discover_v2::V2Page>, String>),
    Nightly(nightly::Event),
    Progress(String, f32),
    /// A pack search's answer, by the number of the search that asked.
    Packs(u64, Result<Vec<sunrise_launcher::browser::Project>, String>),
    /// The categories Modrinth offers for a pack scope; empty when they could not be read.
    PackCategories(pack_discover::Scope, Vec<String>),
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
    /// The registry answered about ratings.
    Rating(rating::Answer),
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
    v2_catalog: V2Catalog,
    v2_catalog_build: Option<Option<String>>,
    local_fit: LocalFitRun,
    /// Why the last read of the v2 index failed, as Installed says it; none while one is out.
    v2_failure: Option<String>,
    find: LineEdit,
    find_focus: Option<FocusHandle>,
    have: LineEdit,
    have_focus: Option<FocusHandle>,
    filters: model::discover_v2::V2Filters,
    menu: Option<Menu>,
    row_filter: model::RowFilter,
    /// The kind Packs › Installed shows.
    shelf: Shelf,
    /// Discover's resource and data pack scopes (`pack_discover`).
    packs: pack_discover::PackState,
    /// The worlds' data packs and the world picker (`data_packs`).
    data: data_packs::DataState,
    selection: BTreeSet<String>,
    /// The last clicked archive path, resolved in the current list for Shift-click.
    anchor: Option<String>,
    snapshot: Option<Snapshot>,
    home_mod_labels: Option<HomeModLabels>,
    /// The mods folder's fingerprint, kept between frames (`native_status::Fingerprints`).
    fingerprints: native_status::Fingerprints,
    /// What the last frame of Installed drew, for the benchmark (`installed_bench_target`).
    drawn: Option<InstalledDrawn>,
    open: Option<Open>,
    page_request: u64,
    nightly: nightly::State,
    /// The collection whose page is open and the members ticked on it.
    v2_picks: Option<collection_v2::V2Picks>,
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
    /// Build was pressed and the build has not started yet.
    pub(crate) build_wanted: bool,
    /// Whether a mod page's Details (who made it, how it is checked) is open.
    /// Discover or Installed, as the player last chose on Mods and on Packs this session.
    last_mods: Option<Page>,
    last_packs: Option<Page>,
    /// The row another page opened Installed at, and whether it was scrolled to yet.
    reveal: Option<search::Reveal>,
    revealed: Rc<std::cell::Cell<bool>>,
    /// A gallery picture enlarged over the window.
    viewer: Option<viewer::Viewer>,
    /// What the refused-Add modal shows (`refused`).
    refused: Option<refused::Refused>,
    /// Registry mods' ratings (`rating`).
    ratings: rating::Ratings,
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
            v2_catalog: V2Catalog::Idle,
            v2_catalog_build: None,
            local_fit: LocalFitRun::default(),
            v2_failure: None,
            find: LineEdit::default(),
            find_focus: None,
            have: LineEdit::default(),
            have_focus: None,
            filters: model::discover_v2::V2Filters::default(),
            menu: None,
            row_filter: model::RowFilter::default(),
            shelf: Shelf::Resource,
            packs: Default::default(),
            data: Default::default(),
            selection: BTreeSet::new(),
            anchor: None,
            snapshot: None,
            home_mod_labels: None,
            fingerprints: Default::default(),
            drawn: None,
            open: None,
            page_request: 0,
            nightly: Default::default(),
            v2_picks: None,
            busy: None,
            notice: None,
            errors: HashMap::new(),
            limit: 0,
            unfit_open: None,
            unfit_details: false,
            build: None,
            build_wanted: false,
            last_mods: None,
            last_packs: None,
            reveal: None,
            revealed: Default::default(),
            viewer: None,
            refused: None,
            ratings: Default::default(),
        }
    }
}

fn package_image(icon: &sunrise_launcher_smod::icon::Icon) -> std::sync::Arc<gpui::Image> {
    gpui::Image::from_bytes(gpui::ImageFormat::Png, icon.bytes().to_vec()).into()
}

/// The mods placed by hand in `home`: every package is opened and read.
fn read_local(home: &Path, archives: &mut HashMap<String, ArchiveDisplay>) -> Vec<model::Local> {
    native_status::untracked(home)
        .into_iter()
        .map(|file| (file, true))
        .chain(
            native_status::untracked_content(home)
                .into_iter()
                .map(|file| (file, false)),
        )
        .map(|(file, native)| {
            let unreadable = file.icon.is_err();
            if let Ok(icon) = file.icon {
                archives.insert(
                    file.file.clone(),
                    ArchiveDisplay {
                        name: file.name.clone(),
                        authors: file.authors,
                        version: file.version.clone(),
                        icon: icon.as_deref().map(package_image),
                    },
                );
            }
            model::Local {
                id: file.id,
                file: file.file,
                name: file.name,
                version: file.version,
                github: file.origin,
                native,
                served_by: file.served_by,
                unreadable,
            }
        })
        .collect()
}

/// The read-only Home view uses the same Installed classification as the full rows.
pub(crate) use model::ManualBadge;
pub(crate) use ui::manual_badge_pill;

/// The small pieces of a mod's Installed row that Home shows without reading its archive again.
pub(crate) struct HomeModFacts {
    pub(crate) kind: &'static str,
    pub(crate) badge: Option<ManualBadge>,
    pub(crate) status: Option<String>,
}

struct HomeModLabels {
    inventory: u64,
    unfit_revision: u64,
    rows: Arc<HashMap<String, HomeModFacts>>,
}

impl HomeModLabels {
    fn matches(&self, inventory: u64, unfit_revision: u64) -> bool {
        self.inventory == inventory && self.unfit_revision == unfit_revision
    }
}

impl Launcher {
    /// The Mods screen for `self.ui.page`: Discover, Installed or a mod's page.
    pub(crate) fn mods(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.mods_drain();
        self.mods_refresh();
        self.refresh_nightly_scope();
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
            Page::Project if self.ui.mods.nightly.collection.is_some() => {
                self.source_collection(cx)
            }
            Page::Project if self.ui.mods.open.is_none() && self.ui.mods.nightly.open.is_some() => {
                self.nightly_project(cx)
            }
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
                    if open.installed.is_some()
                        && (scope.profile != open.scope.profile || scope.home != open.scope.home)
                    {
                        self.ui.page = Page::Installed;
                        return self.installed(window, &have_focus, cx);
                    }
                    if open.v2 {
                        self.mods_load_v2_page(open.id, open.from);
                    } else {
                        self.mods_load_page(open.id, open.from, open.installed);
                    }
                    if let Some(now) = self.ui.mods.open.as_mut() {
                        now.parent = open.parent;
                    }
                }
                let listing = self.ui.mods.open.as_ref().and_then(|open| {
                    let Some(InstalledProject::Local(file)) = &open.installed else {
                        return None;
                    };
                    self.installed_source_project(&open.id, file).cloned()
                });
                if let Some(source) = listing {
                    if self
                        .ui
                        .mods
                        .nightly
                        .open
                        .as_ref()
                        .is_none_or(|open| open.source != source)
                    {
                        self.inspect_nightly_from(source, false, Page::Installed);
                    }
                    self.nightly_project(cx)
                } else {
                    self.ui.mods.nightly.open = None;
                    self.project(cx)
                }
            }
            Page::Installed => self.installed(window, &have_focus, cx),
            Page::PackFind => self.pack_find(window, &find_focus, cx),
            Page::PackProject => self.pack_project(cx),
            _ => self.discover(window, &find_focus, cx),
        }
    }

    /// The exact Sunrise build ID of `instance` for v2 fit checks (unwrapping native-built tags
    /// back to their base release build).
    pub(super) fn profile_exact_build(&self, instance: &instances::Instance) -> Option<String> {
        let raw = self.profile_target(instance)?;
        if native_mods::is_native(&raw) {
            crate::glass_instance::native_base(&raw).or(Some(raw))
        } else {
            Some(raw)
        }
    }

    pub(super) fn selected_exact_build(&self) -> Option<String> {
        let instance = self.selected_instance()?;
        self.profile_exact_build(instance)
    }

    /// Opens a v2 registry mod's page from Discover.
    pub(super) fn mods_show_v2_project(&mut self, id: String) {
        self.ui.mods.nightly.open = None;
        self.ui.mods.nightly.collection = None;
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
        self.ui.page = Page::Project;
        self.mods_load_v2_page(id, from);
    }

    /// Opens a mod's page, from Home or anywhere else. Its details are read in a worker.
    pub(crate) fn mods_show_project(&mut self, id: String) {
        self.mods_show_v2_project(id);
    }

    fn mods_show_installed_project(&mut self, row: &Row) {
        // A signed registry mod has the same page as in Discover, with its installed controls.
        if let Some(entry) = row.entry.as_ref().filter(|_| row.v2_installed()) {
            self.mods_show_v2_project(entry.project.clone());
            return;
        }
        self.ui.mods.nightly.open = None;
        self.ui.mods.nightly.collection = None;
        let (id, installed) = match &row.local {
            Some(local) => (row.id.clone(), InstalledProject::Local(local.file.clone())),
            None => (
                row.entry
                    .as_ref()
                    .map_or(&row.id, |entry| &entry.project)
                    .clone(),
                InstalledProject::Registry,
            ),
        };
        self.ui.page = Page::Project;
        self.mods_load_page(id, Page::Installed, Some(installed));
    }

    /// The page of a mod just removed from `home` has nothing left to show: go back to where it
    /// was opened from.
    pub(crate) fn mods_leave_removed(&mut self, home: &Path, ids: &[String]) {
        if self.ui.page != Page::Project || self.mods_home() != home {
            return;
        }
        let gone = self.ui.mods.open.as_ref().is_some_and(|open| {
            ids.contains(&open.id) || open.removing.iter().any(|id| ids.contains(id))
        });
        if let Some(open) = self.ui.mods.open.take_if(|_| gone) {
            self.ui.page = open.from;
        }
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
        self.build_lines(native_status::build_mods(&self.mods_home()), true)
    }

    pub(crate) fn build_lines(
        &self,
        inputs: Vec<native_status::BuildMod>,
        selected: bool,
    ) -> Vec<(String, String, String, Option<String>)> {
        let rows = if selected { self.rows() } else { Vec::new() };
        inputs
            .into_iter()
            .map(|native_status::BuildMod { id, title, version }| {
                match rows.iter().find(|r| r.id == id && r.version == version) {
                    Some(row) => (id, row.title.clone(), version, row.icon.clone()),
                    None => (id, title, version, None),
                }
            })
            .collect()
    }

    /// Updates the installed mod `id` to the newer version the registry has, as its row does.
    pub(crate) fn update_mod(&mut self, id: &str) {
        if let Some(row) = self.rows().into_iter().find(|r| r.id == id) {
            self.update_one(&row);
        }
    }

    /// Home's inventory can finish before Installed's worker. Leave the labels blank until the
    /// current setup has confirmed rows, and rebuild them on the worker's next publication.
    pub(crate) fn home_mod_facts(&mut self, inventory: u64) -> Arc<HashMap<String, HomeModFacts>> {
        let unfit_revision = sunrise::unfit_updates_revision();
        if let Some(labels) = &self.ui.mods.home_mod_labels {
            if labels.matches(inventory, unfit_revision) {
                return labels.rows.clone();
            }
        }
        self.mods_refresh();
        let mut labels = HashMap::new();
        if self.ui.mods.snapshot.as_ref().is_some_and(Snapshot::ready) {
            for row in self.rows().into_iter().filter(|row| row.on) {
                let error = self.ui.mods.errors.get(&row.id).map(String::as_str);
                let badge = row.manual_badge();
                let cells = model::cells(&row, error, None);
                labels.insert(
                    row.id,
                    HomeModFacts {
                        kind: cells.kind,
                        badge,
                        status: cells.status.map(|state| state.label),
                    },
                );
            }
        }
        let rows = Arc::new(labels);
        self.ui.mods.home_mod_labels = Some(HomeModLabels {
            inventory,
            unfit_revision,
            rows: rows.clone(),
        });
        rows
    }

    pub(crate) fn show_resource_pack_discover(&mut self) {
        self.leave_section(true);
        // Enter Discover with the requested kind, not the shelf we last left open.
        self.ui.mods.shelf = Shelf::Resource;
        // Force one fresh search for the cleared query even when Resource was already selected.
        self.ui.mods.packs.status = pack_discover::Status::Idle;
        self.choose_view(Page::PackFind);
    }

    fn mods_snapshot_key(
        &mut self,
        home: &Path,
        now: std::time::Instant,
    ) -> (String, native_mods::Profile, Option<String>) {
        let profile = self
            .selected_instance()
            .map_or_else(native_mods::Profile::default, |i| i.native_profile);
        let base = self
            .selected_instance()
            .and_then(|i| self.version_of(i).cloned());
        let key = format!(
            "{}\n{}\n{base:?}",
            self.ui.mods.fingerprints.read(home, None, now),
            profile.name(),
        );
        (key, profile, base)
    }

    /// The message pump's look at the mods folder while a Mods page is open: once the kept
    /// fingerprint is due its full read, read it, so a mod rewritten where it lies shows without
    /// waiting for input. True when the page has something new to read; an unchanged folder asks
    /// for no frame.
    pub(crate) fn mods_check_due(&mut self, now: std::time::Instant) -> bool {
        if !matches!(
            self.ui.page,
            Page::Installed | Page::Discover | Page::Project
        ) || !self.ui.mods.fingerprints.due(now)
        {
            return false;
        }
        let home = self.mods_home();
        let (key, _, _) = self.mods_snapshot_key(&home, now);
        self.ui
            .mods
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.home == home && snapshot.key != key)
    }

    /// The play bar's quiet line about them: Mods › Installed, where each says why.
    pub(crate) fn show_installed_mods(&mut self) {
        self.ui.mods.drawn = None;
        self.leave_section(false);
        self.mods_refresh();
        self.choose_view(Page::Installed);
    }

    /// `sunrise-launcher benchmark` with `SUNRISE_LAUNCHER_BENCH_LIST=installed`: how the last
    /// frame left the Installed list, and where a wheel step goes over it.
    pub(crate) fn installed_bench_target(
        &self,
    ) -> (crate::perf::Target, gpui::Bounds<gpui::Pixels>) {
        use crate::perf::Target;
        if self.ui.page != Page::Installed {
            return (Target::Closed, gpui::Bounds::default());
        }
        // What the last frame drew, with the list's handle from that same frame.
        let handle = crate::frame::drawn_scroll("installed");
        let laid = handle.as_ref().map(|handle| {
            (
                f32::from(handle.offset().y),
                f32::from(handle.max_offset().y),
            )
        });
        let bounds = handle.map_or_else(gpui::Bounds::default, |handle| handle.bounds());
        (installed_target(self.ui.mods.drawn, laid), bounds)
    }

    /// Mods › Discover itself, whatever Mods last showed.
    pub(crate) fn show_discover(&mut self) {
        self.leave_section(false);
        self.choose_view(Page::Discover);
    }

    /// The Mods tab: where the player last was this session, else Installed.
    pub(super) fn open_mods(&mut self) {
        self.leave_section(false);
        self.ui.page = self.ui.mods.last_mods.unwrap_or_else(|| {
            self.mods_refresh();
            Page::Installed
        });
    }

    /// The Packs tab: where the player last was this session, else Installed.
    pub(super) fn open_packs(&mut self) {
        self.leave_section(true);
        self.ui.page = self.ui.mods.last_packs.unwrap_or_else(|| {
            self.load_packs_list(self.mods_home());
            Page::Packs
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
        m.nightly.open = None;
        m.nightly.collection = None;
        match page {
            Page::Discover | Page::Installed => m.last_mods = Some(page),
            _ => m.last_packs = Some(page),
        }
        m.selection.clear();
        m.anchor = None;
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

    /// The width a page's own content has: the window less its edges, the sidebar (open, closed or
    /// on its way), the gap beside it, the pane's border and the page's margins.
    pub(super) fn content_width(&self) -> f32 {
        let edges = 2. * crate::tokens::FRAME_INSET + 12.;
        self.window_width - edges - crate::rail::rail_width(self.rail_open()) - 2. - 48.
    }

    pub(crate) fn mods_home(&self) -> PathBuf {
        self.selected_instance()
            .map(|i| i.home.clone())
            .unwrap_or_else(backend::game_home)
    }

    /// What the workers said. Nothing here touches a file.
    pub(super) fn mods_drain(&mut self) -> bool {
        let mut changed = false;
        let mut local_fit_due = false;
        while let Ok(event) = self.ui.mods.rx.try_recv() {
            changed = true;
            let m = &mut self.ui.mods;
            match event {
                Event::V2Catalog(build, result) => {
                    if m.v2_catalog_build.as_ref() == Some(&build) {
                        m.v2_failure = result.as_ref().err().map(|error| match error {
                            sunrise::v2::LoadError::Fetch(_) => trf(
                                "Registry not reachable: {error}",
                                &[("error", &error.to_string())],
                            ),
                            _ => trf(
                                "Registry refused: {error}",
                                &[("error", &error.to_string())],
                            ),
                        });
                        if result.is_ok() {
                            m.home_mod_labels = None;
                        }
                        m.v2_catalog = match result {
                            Ok(payload) => V2Catalog::Ready {
                                cards: payload.cards.into(),
                                collections: payload.collections.into(),
                                categories: payload.categories.into(),
                                build_listed: payload.build_listed,
                                stale_warning: payload.stale_warning,
                                from_cache: payload.from_cache,
                            },
                            Err(error) => V2Catalog::Failed(error.to_string()),
                        };
                        local_fit_due = true;
                    }
                }
                Event::LocalFit(build, result) => {
                    if m.local_fit.build.as_ref() == Some(&build) {
                        m.local_fit.running = false;
                        m.local_fit.failed = result.err();
                    }
                    self.v2_cards_again();
                    local_fit_due = true;
                }
                Event::V2Saved => m.home_mod_labels = None,
                Event::Local(key, result) => {
                    if let Some(snap) = m.snapshot.as_mut().filter(|s| s.key == key) {
                        let ready = snap.finish(*result);
                        m.home_mod_labels = None;
                        if ready {
                            let rows: HashSet<String> =
                                self.rows().into_iter().map(|r| r.id).collect();
                            self.ui.mods.selection.retain(|id| rows.contains(id));
                        }
                    }
                }
                Event::Enabled(home, notice, result) => {
                    self.enabled_mods(&home, notice, result);
                }
                Event::V2Page(request, page) => {
                    if page.is_ok() {
                        self.ui.mods.v2_failure = None;
                    }
                    let scope = self.mods_page_scope();
                    if let Some(open) = self.ui.mods.open.as_mut() {
                        open.finish_v2(request, &scope, page);
                    }
                }
                Event::Nightly(event) => self.nightly_event(event),
                Event::Progress(line, f) => m.busy = Some((line, f)),
                Event::Packs(seq, result) => self.pack_answer(seq, result),
                Event::PackCategories(scope, found) => self.pack_categories(scope, found),
                Event::PackPage(key, page) => self.pack_page_answer(key, page),
                Event::PackDone(result) => self.pack_installed(result),
                Event::Checked(title, found) => self.checked(title, found),
                Event::Rating(answer) => self.rating_answer(answer),
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
        if local_fit_due {
            // A catalog read while a check runs drew its cards without it: draw them again.
            if self.ui.mods.local_fit.running {
                self.v2_cards_again();
            }
            self.start_local_fit();
        }
        changed
    }

    /// This computer's fit checks of `build`, as Discover shows them.
    fn local_fits(
        &self,
        index: &sunrise_launcher_smod::index_v2::Index,
        build: Option<&str>,
    ) -> model::discover_v2::LocalFits {
        let run = &self.ui.mods.local_fit;
        let running = run.running && run.build.as_deref() == build;
        model::discover_v2::LocalFits::read(index, build, running)
    }

    /// Starts this computer's fit check of the releases the index says nothing of on the
    /// selected build, in the background: one at a time, and not again for a build whose last
    /// check failed until Try again.
    fn start_local_fit(&mut self) {
        let run = &self.ui.mods.local_fit;
        let Some(build) = self.selected_exact_build() else {
            return;
        };
        if run.running || (run.build.as_ref() == Some(&build) && run.failed.is_some()) {
            return;
        }
        let Some(catalog) = sunrise::v2::known_index() else {
            return;
        };
        let Some(listed) = catalog.index.build(&build) else {
            return;
        };
        let found = local_fit::kept(&listed.commit);
        if local_fit::gaps(&catalog.index, &build)
            .iter()
            .all(|r| found.contains_key(&r.file.sha256.to_ascii_lowercase()))
        {
            return;
        }
        self.ui.mods.local_fit = LocalFitRun {
            build: Some(build.clone()),
            running: true,
            failed: None,
        };
        let tx = self.ui.mods.tx.clone();
        std::thread::spawn(move || {
            let result = local_fit::check(&catalog.index, &build);
            let _ = tx.send(Event::LocalFit(build, result));
        });
        self.v2_cards_again();
    }

    /// Try again after the selected build's fit check failed.
    pub(super) fn retry_local_fit(&mut self) {
        self.ui.mods.local_fit.failed = None;
        self.start_local_fit();
    }

    /// Why the selected build's fit check failed, when it did.
    pub(super) fn local_fit_failure(&self) -> Option<&str> {
        let run = &self.ui.mods.local_fit;
        (run.build == self.selected_exact_build())
            .then_some(run.failed.as_deref())
            .flatten()
    }

    /// Discover's cards and an open mod page drawn again from the index in memory, with what
    /// this computer's checks have found so far.
    fn v2_cards_again(&mut self) {
        let Some(catalog) = sunrise::v2::known_index() else {
            return;
        };
        let build = self.selected_exact_build();
        let local = self.local_fits(&catalog.index, build.as_deref());
        let name = &crate::sheets::build_name;
        if self.ui.mods.v2_catalog_build.as_ref() == Some(&build) {
            if let V2Catalog::Ready {
                cards, collections, ..
            } = &mut self.ui.mods.v2_catalog
            {
                let fresh =
                    model::discover_v2::cards(&catalog.index, build.as_deref(), name, &local);
                *collections = model::discover_v2::collections(&catalog.index, &fresh).into();
                *cards = fresh.into();
            }
        }
        if let Some(Ok(page)) = self
            .ui
            .mods
            .open
            .as_mut()
            .and_then(|open| open.v2_page.as_mut())
        {
            let id = page.id.clone();
            if let Some(fresh) =
                model::discover_v2::page(&catalog.index, &id, build.as_deref(), name, &local)
            {
                **page = fresh;
            }
        }
    }

    /// Reads what is installed again when the files' fingerprint moved (an install, a toggle or a
    /// build elsewhere), and asks for the registry once. Reads only.
    pub(crate) fn mods_refresh(&mut self) {
        let home = self.mods_home();
        self.mods
            .follow_home(self.selected_instance().map(|i| i.home.clone()));
        // Asked on every frame of the Mods pages: read again only when something moved.
        let (key, profile, base) = self.mods_snapshot_key(&home, std::time::Instant::now());
        let moved = self
            .ui
            .mods
            .snapshot
            .as_ref()
            .is_some_and(|s| s.home != home);
        if moved {
            // Another setup: nothing reviewed or selected for the old one carries over.
            let m = &mut self.ui.mods;
            m.selection.clear();
            m.anchor = None;
            m.errors.clear();
            m.notice = None;
            self.pack_setup_moved();
        }
        if self.ui.mods.snapshot.as_ref().is_none_or(|s| s.key != key) {
            self.ui.mods.home_mod_labels = None;
            // Keep the same profile's confirmed rows and advisory until the worker answers.
            let (rows, backup_notice) = self
                .ui
                .mods
                .snapshot
                .take()
                .filter(|s| s.home == home)
                .map(|s| (s.rows, s.backup_notice))
                .unwrap_or_default();
            let (tx, worker_home, worker_key, worker_base) =
                (self.ui.mods.tx.clone(), home.clone(), key.clone(), base);
            std::thread::spawn(move || {
                let result = InstalledRows::read(&worker_home, profile, worker_base.as_deref());
                if matches!(result, Ok(None)) {
                    // Delay just the busy display retry, without holding any profile/cache lock.
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                let _ = tx.send(Event::Local(worker_key, Box::new(result)));
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
        let snapshot = self.ui.mods.snapshot.as_ref();
        if matches!(self.ui.page, Page::Installed | Page::Project)
            && snapshot.is_some_and(|s| {
                s.rows.local.iter().any(|local| {
                    local
                        .github
                        .as_ref()
                        .is_some_and(|origin| origin.source.is_some())
                })
            })
        {
            self.read_nightly_catalogue(false);
        }
        let has_registry_rows = self
            .ui
            .mods
            .snapshot
            .as_ref()
            .is_some_and(|s| !s.rows.entries.is_empty());
        if has_registry_rows || matches!(self.ui.page, Page::Discover | Page::Project) {
            self.ensure_v2_catalog(false);
        }
    }

    pub(super) fn ensure_v2_catalog(&mut self, force: bool) {
        let build = self.selected_exact_build();
        if !force
            && self.ui.mods.v2_catalog_build.as_ref() == Some(&build)
            && !matches!(self.ui.mods.v2_catalog, V2Catalog::Idle)
        {
            return;
        }
        if let Some(catalog) = (!force).then(sunrise::v2::known_index).flatten() {
            let local = self.local_fits(&catalog.index, build.as_deref());
            let cards = model::discover_v2::cards(
                &catalog.index,
                build.as_deref(),
                &crate::sheets::build_name,
                &local,
            );
            let categories = model::discover_v2::categories(&cards);
            let collections = model::discover_v2::collections(&catalog.index, &cards);
            let build_listed = model::discover_v2::build_listed(&catalog.index, build.as_deref());
            self.ui.mods.v2_failure = None;
            self.ui.mods.v2_catalog_build = Some(build);
            self.ui.mods.v2_catalog = V2Catalog::Ready {
                cards: cards.into(),
                collections: collections.into(),
                categories: categories.into(),
                build_listed,
                stale_warning: catalog.stale_warning,
                from_cache: catalog.from_cache,
            };
            self.start_local_fit();
            return;
        }
        self.ui.mods.v2_catalog_build = Some(build.clone());
        self.ui.mods.v2_catalog = V2Catalog::Loading;
        self.ui.mods.v2_failure = None;
        let tx = self.ui.mods.tx.clone();
        std::thread::spawn(move || {
            // Installed starts from the snapshot on disk while the network read is out.
            if !force && sunrise::v2::read_saved_index().is_some() {
                let _ = tx.send(Event::V2Saved);
            }
            let loaded = if force {
                sunrise::v2::refreshed_index()
            } else {
                sunrise::v2::index()
            };
            let result = loaded.map(|catalog| {
                // What a running check finds shows once it answers.
                let local =
                    model::discover_v2::LocalFits::read(&catalog.index, build.as_deref(), false);
                let cards = model::discover_v2::cards(
                    &catalog.index,
                    build.as_deref(),
                    &crate::sheets::build_name,
                    &local,
                );
                let categories = model::discover_v2::categories(&cards);
                let collections = model::discover_v2::collections(&catalog.index, &cards);
                V2CatalogPayload {
                    collections,
                    build_listed: model::discover_v2::build_listed(
                        &catalog.index,
                        build.as_deref(),
                    ),
                    cards,
                    categories,
                    stale_warning: catalog.stale_warning,
                    from_cache: catalog.from_cache,
                }
            });
            let _ = tx.send(Event::V2Catalog(build, result));
        });
    }

    fn mods_page_scope(&self) -> OpenScope {
        let instance = self.selected_instance();
        OpenScope {
            profile: instance.map(|i| i.id.clone()),
            home: self.mods_home(),
            target: instance.and_then(|i| self.profile_target(i)),
            game: self.mods_game(),
        }
    }

    /// An installed mod's page that the signed registry doesn't list (a local archive, or one
    /// from the old registry): drawn from its installed row, with nothing to fetch.
    fn mods_load_page(&mut self, id: String, from: Page, installed: Option<InstalledProject>) {
        crate::frame::scrolls("project").set_offset(gpui::point(px(0.), px(0.)));
        let scope = self.mods_page_scope();
        self.ui.mods.page_request += 1;
        let request = self.ui.mods.page_request;
        self.ui.mods.open = Some(Open {
            id,
            installed,
            collection: false,
            v2: false,
            request,
            scope,
            from,
            v2_page: None,
            parent: None,
            removing: Vec::new(),
        });
    }

    fn mods_load_v2_page(&mut self, id: String, from: Page) {
        crate::frame::scrolls("project").set_offset(gpui::point(px(0.), px(0.)));
        let scope = self.mods_page_scope();
        let exact_build = self.selected_exact_build();
        self.ui.mods.page_request += 1;
        let request = self.ui.mods.page_request;
        let immediate = sunrise::v2::known_index().map(|catalog| {
            let local = self.local_fits(&catalog.index, exact_build.as_deref());
            model::discover_v2::page(
                &catalog.index,
                &id,
                exact_build.as_deref(),
                &crate::sheets::build_name,
                &local,
            )
            .map(Box::new)
            .ok_or_else(|| trf("{id} isn't a mod in Sunrise's registry.", &[("id", &id)]))
        });
        let needs_fetch = immediate.is_none();
        self.ui.mods.open = Some(Open {
            id: id.clone(),
            installed: None,
            collection: false,
            v2: true,
            request,
            scope,
            from,
            v2_page: immediate,
            parent: None,
            removing: Vec::new(),
        });
        self.ratings_for_page(&id);
        if !needs_fetch {
            return;
        }
        let tx = self.ui.mods.tx.clone();
        std::thread::spawn(move || {
            let result = sunrise::v2::index()
                .map_err(|e| e.to_string())
                .and_then(|catalog| {
                    let local = model::discover_v2::LocalFits::read(
                        &catalog.index,
                        exact_build.as_deref(),
                        false,
                    );
                    model::discover_v2::page(
                        &catalog.index,
                        &id,
                        exact_build.as_deref(),
                        &crate::sheets::build_name,
                        &local,
                    )
                    .map(Box::new)
                    .ok_or_else(|| trf("{id} isn't a mod in Sunrise's registry.", &[("id", &id)]))
                });
            let _ = tx.send(Event::V2Page(request, result));
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
        let unfit_updates = snap
            .rows
            .managed
            .as_ref()
            .map_or_else(BTreeSet::new, |managed| {
                sunrise::known_unfit_updates(managed.base())
            });
        let v2 = sunrise::v2::shown_index();
        let build = snap
            .rows
            .managed
            .as_ref()
            .map(|managed| managed.base().to_owned())
            .or_else(|| self.selected_exact_build());
        let mut rows = model::rows(
            &snap.rows.entries,
            &model::World {
                v2: v2.as_ref().map(|catalog| &*catalog.index),
                v2_failure: self.ui.mods.v2_failure.as_deref(),
                build: build.as_deref(),
                unfit_updates: &unfit_updates,
                enabled: &snap.rows.enabled,
                unbuilt: &snap.rows.unbuilt,
                chosen: &snap.rows.chosen,
                local: &snap.rows.local,
            },
        );
        let mut regroup = false;
        for row in &mut rows {
            let held = snap.rows.switch_held.get(&row.id);
            if let Some(reason) = held {
                row.held_for = Some(model::Hold {
                    build: crate::sheets::build_name(reason.build()),
                    reason: reason.clone(),
                    retry: snap.rows.local_holds.contains(&row.id),
                });
                row.on = false;
                if !matches!(row.need, Some(model::Need::TakenDown(_))) {
                    row.need = None;
                }
                regroup = true;
            }
        }
        // A mod the game was made without isn't waiting for a build: it waits for its author.
        let left = if snap.rows.managed.is_none() {
            self.native_left_out()
        } else {
            Vec::new()
        };
        if !left.is_empty() {
            let on = self
                .selected_instance()
                .and_then(|i| self.version_of(i))
                .map(|tag| crate::sheets::build_name(tag))
                .unwrap_or_default();
            model::mark_left_out(&mut rows, &left, &on, crate::sheets::build_name);
            regroup = true;
        }
        if regroup {
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

    /// Build… on a row: the one native build path. The player stays where they are: the play bar shows how far it is.
    fn build(&mut self) {
        self.build_native();
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
            (false, false) => tr("Search mods…"),
            (false, true) => tr("Search your mods…"),
            (true, false) => match self.ui.mods.packs.scope {
                pack_discover::Scope::Resource => tr("Search resource packs…"),
                pack_discover::Scope::Shader => tr("Search shader packs…"),
                pack_discover::Scope::Data => tr("Search data packs…"),
            },
            (true, true) => match self.ui.mods.shelf {
                Shelf::Resource => tr("Search your resource packs…"),
                Shelf::Shader => tr("Search your shader packs…"),
                Shelf::Data => tr("Search your data packs…"),
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
                            m.limit = 0;
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
        let sub_tabs = div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_x(px(16.))
            .gap_y(px(8.))
            .border_b_1()
            .border_color(gpui::rgb(palette().divider))
            .child(segmented(
                "alt-mods-tabs",
                [
                    (
                        "alt-mods-tab-installed",
                        tr("Installed").into(),
                        installed,
                        if packs { Page::Packs } else { Page::Installed },
                    ),
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
                ],
                Launcher::choose_view,
                cx,
            ));
        let toolbar = if installed && !packs {
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.))
                .child(field)
                .child(self.add_from_source_button(cx))
                .into_any_element()
        } else {
            div().flex().items_center().child(field).into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(sub_tabs)
            .child(toolbar)
    }
}

#[cfg(test)]
mod enabled_tests {
    use super::*;

    #[test]
    fn reopening_the_same_mod_discards_previous_detail_replies() {
        let scope = |profile: &str, target: &str, game: &str| OpenScope {
            profile: Some(profile.into()),
            home: PathBuf::from(profile),
            target: Some(target.into()),
            game: game.into(),
        };
        let current = scope("current", "new-build", "26.3");
        let mut open = Open {
            id: "ruby".into(),
            installed: Some(InstalledProject::Registry),
            collection: false,
            v2: false,
            request: 2,
            scope: scope("current", "new-build", "26.3"),
            from: Page::Installed,
            v2_page: None,
            parent: None,
            removing: Vec::new(),
        };
        open.finish_v2(1, &current, Err("earlier request".into()));
        open.finish_v2(
            2,
            &scope("old", "new-build", "26.3"),
            Err("old profile".into()),
        );
        open.finish_v2(
            2,
            &scope("current", "old-build", "26.3"),
            Err("old build".into()),
        );
        open.finish_v2(
            2,
            &scope("current", "new-build", "26.2"),
            Err("old game".into()),
        );
        assert!(open.v2_page.is_none());
        open.finish_v2(2, &current, Err("current reply".into()));
        open.finish_v2(1, &current, Err("earlier request".into()));
        assert!(matches!(open.v2_page, Some(Err(why)) if why == "current reply"));
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
            file: format!("{id}.smod"),
            name: id.into(),
            version: String::new(),
            github: None,
            native: false,
            served_by: None,
            unreadable: false,
        };
        // "gone" was a package of the list known before and is no longer there.
        let got = enabled_ids(home.path(), &[], &[local("kept"), local("off")]);
        assert_eq!(got, HashSet::from(["kept".to_string()]));
    }
}

#[cfg(test)]
#[path = "mods/home_tests.rs"]
mod home_tests;

#[cfg(test)]
mod installed_refresh_tests {
    use super::*;

    #[test]
    fn the_benchmark_times_installed_only_once_rows_read_afresh_are_drawn() {
        use crate::perf::Target;
        let laid = Some((-10., 400.));
        // A refresh draws the last read's rows while it reads: they are waited past, not timed.
        let retained = InstalledDrawn::of(true, false, 12);
        assert_eq!(retained, InstalledDrawn::Reading);
        assert_eq!(installed_target(Some(retained), laid), Target::LayingOut);
        // Then the fresh rows: timed from the frame that drew them.
        let fresh = InstalledDrawn::of(false, false, 14);
        assert_eq!(fresh, InstalledDrawn::Rows);
        assert_eq!(
            installed_target(Some(fresh), laid),
            Target::Laid {
                offset: -10.,
                max: 400.
            }
        );
        assert_eq!(installed_target(Some(fresh), None), Target::LayingOut);
        // An empty profile draws no list: it stops at once, saying so, not at the timeout.
        let empty = InstalledDrawn::of(false, false, 0);
        assert_eq!(empty, InstalledDrawn::Empty);
        assert_eq!(installed_target(Some(empty), None), Target::Empty);
        // A failed read over the rows from before it stops too, with its own reason.
        let failed = InstalledDrawn::of(false, true, 12);
        assert_eq!(failed, InstalledDrawn::Failed);
        assert_eq!(installed_target(Some(failed), laid), Target::Failed);
        // Not drawn yet.
        assert_eq!(installed_target(None, laid), Target::LayingOut);
    }

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
                        detail: None,
                        build: "fixture".into(),
                    },
                )]),
                ..Default::default()
            },
            reading: true,
            error: None,
            backup_notice: None,
        };
        assert!(!snapshot.finish(InstalledRows::read(
            &home,
            native_mods::Profile::default(),
            None,
        )));
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
        assert!(snapshot.finish(InstalledRows::read(
            &home,
            native_mods::Profile::default(),
            None,
        )));
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
        snapshot.finish(InstalledRows::read(
            home.path(),
            native_mods::Profile::default(),
            None,
        ));
        assert!(snapshot.enabled.contains("ruby"));
        assert!(matches!(
            InstalledRefresh::of(Some(&snapshot)),
            InstalledRefresh::Failed(InstalledReadError::GameInstall(_), _)
        ));
        assert_eq!(std::fs::read(&journal).unwrap(), b"not a journal");
        std::fs::remove_file(journal).unwrap();
        snapshot.finish(InstalledRows::read(
            home.path(),
            native_mods::Profile::default(),
            None,
        ));
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

    #[test]
    fn home_mod_labels_invalidate_when_unfit_updates_revision_changes() {
        let cached = HomeModLabels {
            inventory: 7,
            unfit_revision: 3,
            rows: Arc::new(HashMap::new()),
        };
        assert!(cached.matches(7, 3));
        assert!(
            !cached.matches(7, 4),
            "a newly recorded unfit update must refresh Home's mod status even with unchanged inventory"
        );
    }
}
