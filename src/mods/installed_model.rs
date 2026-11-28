//! The installed list's model: rows read from the folder, their on/off state, selection
//! fields, refresh snapshot and the pure helpers over them.

use super::*;

/// The ids switched on among the registry entries and the hand-placed packages.
pub(super) fn enabled_ids(
    home: &Path,
    entries: &[Entry],
    local: &[model::Local],
) -> HashSet<String> {
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

pub(super) struct ArchiveDisplay {
    pub(super) name: String,
    pub(super) authors: Vec<String>,
    pub(super) version: String,
    pub(super) icon: Option<std::sync::Arc<gpui::Image>>,
}

#[derive(Default)]
pub(super) struct InstalledRows {
    pub(super) entries: Vec<Entry>,
    pub(super) archives: HashMap<String, ArchiveDisplay>,
    pub(super) enabled: HashSet<String>,
    pub(super) chosen: std::collections::BTreeMap<String, String>,
    pub(super) local: Vec<model::Local>,
    pub(super) managed: Option<Box<sunrise_launcher::managed_play::Active>>,
    pub(super) unbuilt: HashSet<String>,
    pub(super) switch_held:
        std::collections::HashMap<String, sunrise_launcher::managed_play::HeldReason>,
    /// Held local or source mods: their switch checks their exact bytes again.
    pub(super) local_holds: HashSet<String>,
}

#[derive(Debug)]
pub(super) enum InstalledReadError {
    GameInstall(installed::GameInstallProblem),
    Other(String),
}

pub(super) type InstalledRead = Result<Option<(InstalledRows, Option<String>)>, InstalledReadError>;

impl InstalledRows {
    pub(super) fn read(
        home: &Path,
        profile: native_mods::Profile,
        base: Option<&str>,
    ) -> InstalledRead {
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
    pub(super) fn of(reading: bool, failed: bool, rows: usize) -> Self {
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
pub(super) fn installed_target(
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

pub(super) struct Snapshot {
    pub(super) key: String,
    pub(super) home: PathBuf,
    pub(super) rows: InstalledRows,
    pub(super) reading: bool,
    pub(super) error: Option<InstalledReadError>,
    pub(super) backup_notice: Option<String>,
}

impl std::ops::Deref for Snapshot {
    type Target = InstalledRows;

    fn deref(&self) -> &Self::Target {
        &self.rows
    }
}

impl Snapshot {
    pub(super) fn finish(&mut self, result: InstalledRead) -> bool {
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

    pub(super) fn ready(&self) -> bool {
        !self.reading && self.error.is_none()
    }
}

pub(super) enum InstalledRefresh<'a> {
    Reading,
    Failed(&'a InstalledReadError, &'a Path),
    Ready,
}

impl InstalledRefresh<'_> {
    pub(super) fn of(snapshot: Option<&Snapshot>) -> InstalledRefresh<'_> {
        match snapshot {
            Some(snapshot) if !snapshot.reading => match &snapshot.error {
                Some(error) => InstalledRefresh::Failed(error, &snapshot.home),
                None => InstalledRefresh::Ready,
            },
            _ => InstalledRefresh::Reading,
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

/// The small pieces of a mod's Installed row that Home shows without reading its archive again.
pub(crate) struct HomeModFacts {
    pub(crate) kind: &'static str,
    pub(crate) badge: Option<ManualBadge>,
    pub(crate) status: Option<String>,
}

pub(super) struct HomeModLabels {
    pub(super) inventory: u64,
    pub(super) unfit_revision: u64,
    pub(super) rows: Arc<HashMap<String, HomeModFacts>>,
}

impl HomeModLabels {
    pub(super) fn matches(&self, inventory: u64, unfit_revision: u64) -> bool {
        self.inventory == inventory && self.unfit_revision == unfit_revision
    }
}

impl Launcher {
    pub(crate) fn registry_install_hold(&self, id: &str) -> Option<installed::InstallHold> {
        let snap = self
            .mods_state
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
        (installed_target(self.mods_state.drawn, laid), bounds)
    }
}
