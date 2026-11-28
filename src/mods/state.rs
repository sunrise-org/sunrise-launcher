//! The Mods page's state: what Discover, Installed and a mod's page remember between
//! frames, and the updates that keep it.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Menu {
    Category,
    Sort,
    /// The "Adding to" profile picker.
    Profile,
}

#[derive(PartialEq, Eq)]
pub(super) struct OpenScope {
    pub(super) profile: Option<String>,
    pub(super) home: PathBuf,
    pub(super) target: Option<String>,
    pub(super) game: String,
}

pub(super) enum InstalledProject {
    Registry,
    Local(String),
}

pub(super) struct Open {
    pub(super) id: String,
    pub(super) installed: Option<InstalledProject>,
    pub(super) collection: bool,
    pub(super) v2: bool,
    pub(super) request: u64,
    pub(super) scope: OpenScope,
    /// The screen Back returns to.
    pub(super) from: Page,
    pub(super) v2_page: Option<Result<Box<model::discover_v2::V2Page>, String>>,
    /// The page this one was opened from, when it is not `from` itself: Back returns to it.
    pub(super) parent: Option<Parent>,
    /// The installed mods this page's Remove asked to remove, recorded while their rows are
    /// still read: the page leaves once that Remove finishes.
    pub(super) removing: Vec<String>,
}

/// A page a mod's page was opened from, which Back returns to before `Open::from`.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Parent {
    /// A registry collection's page (by id), opened from one of its mods.
    Collection(String),
    /// Another mod's page, opened from its Dependencies. It keeps its own way back.
    Project {
        id: String,
        name: String,
        parent: Option<Box<Parent>>,
    },
}

impl Open {
    /// This page as the way back from a dependency opened on it: a registry mod's page, or
    /// nothing for a collection's or an installed archive's, which list no dependencies.
    pub(super) fn as_parent(&self) -> Option<Parent> {
        if !self.v2 || self.collection {
            return None;
        }
        let name = match &self.v2_page {
            Some(Ok(page)) => page.name.clone(),
            _ => self.id.clone(),
        };
        Some(Parent::Project {
            id: self.id.clone(),
            name,
            parent: self.parent.clone().map(Box::new),
        })
    }

    pub(super) fn finish_v2(
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

/// The kinds of pack Packs › Installed lists, one at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Shelf {
    Resource,
    Shader,
    /// Data packs, which live in worlds (`data_packs`).
    Data,
}

impl Shelf {
    /// The setup's own collection it lists; none for data packs.
    pub(super) fn kind(self) -> Option<sunrise_launcher::packs::Kind> {
        match self {
            Shelf::Resource => Some(sunrise_launcher::packs::Kind::Resource),
            Shelf::Shader => Some(sunrise_launcher::packs::Kind::Shader),
            Shelf::Data => None,
        }
    }
}

pub(crate) struct ModsState {
    pub(super) tx: Sender<Event>,
    pub(super) rx: Receiver<Event>,
    pub(super) v2_catalog: V2Catalog,
    pub(super) v2_catalog_build: Option<Option<String>>,
    pub(super) local_fit: LocalFitRun,
    /// Why the last read of the v2 index failed, as Installed says it; none while one is out.
    pub(super) v2_failure: Option<String>,
    pub(super) find: LineEdit,
    pub(super) find_focus: Option<FocusHandle>,
    pub(super) have: LineEdit,
    pub(super) have_focus: Option<FocusHandle>,
    pub(super) filters: model::discover_v2::V2Filters,
    pub(super) menu: Option<Menu>,
    pub(super) row_filter: model::RowFilter,
    /// The kind Packs › Installed shows.
    pub(super) shelf: Shelf,
    /// Discover's resource and data pack scopes (`pack_discover`).
    pub(super) packs: pack_discover::PackState,
    /// The worlds' data packs and the world picker (`data_packs`).
    pub(super) data: data_packs::DataState,
    pub(super) selection: BTreeSet<String>,
    /// The shelf's ticked packs, by file.
    pub(super) pack_selection: super::selection::Selection,
    /// A bulk change was started from Home, where no notice shows: say its result as a toast.
    pub(super) bulk_toast: bool,
    /// The last clicked archive path, resolved in the current list for Shift-click.
    pub(super) anchor: Option<String>,
    pub(super) snapshot: Option<Snapshot>,
    pub(super) home_mod_labels: Option<HomeModLabels>,
    /// The mods folder's fingerprint, kept between frames (`native_status::Fingerprints`).
    pub(super) fingerprints: native_status::Fingerprints,
    /// "Check for updates" on Home's Mods tab, and the sheet of what it found.
    pub(crate) update_check: super::updates::Check,
    pub(super) open: Option<Open>,
    pub(super) page_request: u64,
    pub(super) nightly: nightly::State,
    /// The collection whose page is open and the members ticked on it.
    pub(super) v2_picks: Option<collection_v2::V2Picks>,
    /// My own batch (update or remove): its line and fraction.
    pub(super) busy: Option<(String, f32)>,
    pub(super) notice: Option<Result<String, String>>,
    /// Why a mod's last update failed, shown on its row until the next attempt.
    pub(super) errors: HashMap<String, String>,
    /// How many cards Discover draws ("Show more" adds a page).
    pub(super) limit: usize,
    /// The Installed row whose "?" is open (a mod not on this build), and whether its
    /// technical details show.
    pub(super) unfit_open: Option<String>,
    pub(super) unfit_details: bool,
    /// The build the build window shows: running, or ended and not yet closed.
    pub(crate) build: Option<build_run::Run>,
    /// Build was pressed and the build has not started yet.
    pub(crate) build_wanted: bool,
    /// A gallery picture enlarged over the window.
    pub(super) viewer: Option<viewer::Viewer>,
    /// What the refused-Add modal shows (`refused`).
    pub(super) refused: Option<refused::Refused>,
    /// The release whose builds window is open (`release_builds`).
    pub(super) release_builds: Option<model::discover_v2::ReleaseBuilds>,
    /// Registry mods' ratings (`rating`).
    pub(super) ratings: rating::Ratings,
}

impl ModsState {
    /// Shuts the open filter dropdown.
    pub(crate) fn close_menu(&mut self) {
        self.menu = None;
    }

    pub(super) fn visible_notice(
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
            pack_selection: Default::default(),
            bulk_toast: false,
            anchor: None,
            snapshot: None,
            home_mod_labels: None,
            fingerprints: Default::default(),
            update_check: Default::default(),
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
            viewer: None,
            refused: None,
            release_builds: None,
            ratings: Default::default(),
        }
    }
}

impl Launcher {
    /// Reads what is installed again when the files' fingerprint moved (an install, a toggle or a
    /// build elsewhere), and asks for the registry once. Reads only.
    pub(crate) fn mods_refresh(&mut self) {
        let home = self.mods_home();
        self.mods
            .follow_home(self.selected_instance().map(|i| i.home.clone()));
        // Asked on every frame of the Mods pages: read again only when something moved.
        let key = self
            .mods_state
            .fingerprints
            .read(&home, None, std::time::Instant::now());
        let moved = self
            .mods_state
            .snapshot
            .as_ref()
            .is_some_and(|s| s.home != home);
        if moved {
            // Another setup: nothing reviewed or selected for the old one carries over.
            let m = &mut self.mods_state;
            m.selection.clear();
            m.anchor = None;
            m.errors.clear();
            m.notice = None;
            self.pack_setup_moved();
        }
        if self
            .mods_state
            .snapshot
            .as_ref()
            .is_none_or(|s| s.key != key)
        {
            if moved {
                self.mods_state.home_mod_labels = None;
            }
            // Keep the same profile's confirmed rows and advisory until the worker answers.
            let (rows, backup_notice) = self
                .mods_state
                .snapshot
                .take()
                .filter(|s| s.home == home)
                .map(|s| (s.rows, s.backup_notice))
                .unwrap_or_default();
            let (tx, worker_home, worker_key) =
                (self.mods_state.tx.clone(), home.clone(), key.clone());
            std::thread::spawn(move || {
                let result = InstalledRows::read(&worker_home);
                if matches!(result, Ok(None)) {
                    // Delay just the busy display retry, without holding any profile/cache lock.
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                let _ = tx.send(Event::Local(worker_key, Box::new(result)));
            });
            self.mods_state.snapshot = Some(Snapshot {
                key,
                home,
                rows,
                reading: true,
                error: None,
                backup_notice,
            });
        }
        let snapshot = self.mods_state.snapshot.as_ref();
        if matches!(self.ui.page, Page::Home | Page::Project)
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
            .mods_state
            .snapshot
            .as_ref()
            .is_some_and(|s| !s.rows.entries.is_empty());
        if has_registry_rows || matches!(self.ui.page, Page::Discover | Page::Project) {
            self.ensure_v2_catalog(false);
        }
    }

    pub(super) fn mods_page_scope(&self) -> OpenScope {
        let instance = self.selected_instance();
        OpenScope {
            profile: instance.map(|i| i.id.clone()),
            home: self.mods_home(),
            target: instance.and_then(|i| self.profile_target(i)),
            game: self.mods_game(),
        }
    }

    pub(crate) fn mods_home(&self) -> PathBuf {
        self.selected_instance()
            .map(|i| i.home.clone())
            .unwrap_or_else(backend::game_home)
    }

    /// Whether another install or update holds the folder right now.
    pub(super) fn mods_busy(&self) -> bool {
        self.mutating()
    }

    /// The Mods page's own change under way (a Modrinth mod or pack, a data pack, mods turned
    /// on): its line and how far it is, for the status strip.
    pub(crate) fn mods_change(&self) -> Option<(&str, f32)> {
        // A switch flipped on a Home card shows itself: a strip sliding in and out of the page
        // for it would move everything under it.
        if self.ui.home.tabs.quiet && self.ui.page == Page::Home {
            return None;
        }
        let (line, fraction) = self.mods_state.busy.as_ref()?;
        Some((line.as_str(), *fraction))
    }

    /// Whether a mod change is in progress (mine or the shared install's), for the shared Play
    /// and setup gates. Reads only the Mods busy flags, never the launch state.
    pub(crate) fn mutating(&self) -> bool {
        self.mods_state.busy.is_some() || self.registry_stage().is_some()
    }

    /// A backup advisory should never eclipse either operation's own result.
    pub(super) fn notice(&self) -> Option<Result<String, String>> {
        self.mods_state.visible_notice(self.mods.message.as_ref())
    }

    pub(super) fn dismiss_notice(&mut self) {
        if self.mods_state.notice.is_none() && self.mods.message.is_none() {
            if let Some(snapshot) = &mut self.mods_state.snapshot {
                snapshot.backup_notice = None;
            }
        }
        self.mods_state.notice = None;
        self.mods.message = None;
    }
}
