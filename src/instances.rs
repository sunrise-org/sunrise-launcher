//! Instances: separate game folders, each with its own worlds, options and packs.
//!
//! Nothing that exists moves. The game's home (`backend::game_home`) stays the default instance,
//! "Sunrise", with its saves and options where they are. Imported modpacks (`ports/<pack>/home/`)
//! and registry modpacks (`sunrise-packs/<id>/home/`) are instances too, played from their own
//! folders as before. New instances get `instances/<slug>/` in the game's home. What the player
//! chooses about an instance (its name, icon, group) is kept in `instances.json` in the
//! launcher's data folder, so renaming one never moves a folder.
use crate::backend;
use crate::browser::sunrise::pack as registry_pack;
use crate::launch::Launch;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

mod create;
mod delete;
mod deletion_recovery;
mod edit;
pub(crate) mod lifecycle;
mod list;
mod play;
use crate::managed_play::{Profile as ManagedProfile, ReferencePublication};
use crate::profile_txn::{durable, Identity, NameGate};
pub use create::*;
pub use delete::*;
pub use deletion_recovery::{interrupted_deletions, InterruptedDeletion};
pub use edit::*;
use lifecycle::{initial_choices, GATES, INITIAL, RETIRED};
#[cfg(test)]
use list::list_snapshot_until;
pub use list::*;
pub use play::*;

/// The default instance's id: the game's home itself.
pub const DEFAULT: &str = "sunrise";
const FILE: &str = "instances.json";
/// Where new instances live, in the game's home.
const OWN: &str = "instances";
/// Where a new instance is put together, in the game's home (the same disk as [`OWN`], so
/// moving it there is one rename). Never listed: an instance exists once it is whole.
const STAGING: &str = ".new-instance";

/// The most a picture drawn as artwork (a profile's, a world's, a texture) is read: one a game
/// or the launcher wrote is far smaller.
pub const MAX_ARTWORK: u64 = 8 << 20;

/// The file at `path` for drawing, or None when it is no regular file or is over
/// [`MAX_ARTWORK`]: nothing larger is ever read into memory, whatever its length claims.
pub fn read_artwork(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let mut open = fs::OpenOptions::new();
    open.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // Never wait for a writer if an icon path names a pipe, even if it's replaced
        // immediately before this open. This worker also serves the rest of the launcher.
        open.custom_flags(libc::O_NONBLOCK);
    }
    let file = open.open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > MAX_ARTWORK {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_ARTWORK + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= MAX_ARTWORK).then_some(bytes)
}

/// An instance's own picture, in its game folder: one the player chose, or its modpack's icon.
/// Drawn instead of [`Meta::icon`] while it is there.
pub const PICTURE: &str = "icon.png";
/// How many pixels a side a picture is kept at: twice the largest it is drawn.
pub const PICTURE_SIDE: u32 = 128;

/// Any image file's `bytes` as a picture: PNG, scaled down to fit [`PICTURE_SIDE`].
pub fn picture(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    crate::browser::images::fit_png(bytes, PICTURE_SIDE, 8192, 256 << 20).ok_or_else(|| {
        crate::i18n::tr("That file isn't a picture the launcher can read (PNG, JPEG, GIF, WebP).")
            .into()
    })
}

/// Keeps `png` (made by [`picture`]) as the picture of the instance playing in `home`, or, with
/// None, gives its picture to `to_trash`.
pub fn set_picture(
    home: &Path,
    png: Option<&[u8]>,
    to_trash: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let target = home.join(PICTURE);
    let Some(png) = png else {
        return if target.is_file() {
            to_trash(&target)
        } else {
            Ok(())
        };
    };
    fs::create_dir_all(home).map_err(|e| format!("{}: {e}", home.display()))?;
    let part = home.join("icon.png.part");
    fs::write(&part, png).map_err(|e| format!("{}: {e}", part.display()))?;
    fs::rename(&part, &target).map_err(|e| format!("{}: {e}", target.display()))
}

/// Makes the icon at `url` (a modpack's, from the source it was installed from) the picture of the
/// instance playing in `home`, unless it has one already. Best effort: a pack without its icon
/// still plays.
pub fn adopt_icon(home: &Path, url: &str) {
    if home.join(PICTURE).exists() {
        return;
    }
    let image = crate::browser::images::fetch(&backend::data_dir(), url, PICTURE_SIDE);
    let png = image
        .ok()
        .and_then(|f| fs::read(f).ok())
        .and_then(|b| picture(b).ok());
    if let Some(png) = png {
        let _ = set_picture(home, Some(&png), &|_| Ok(()));
    }
}

pub use sunrise_launcher_domain::{Channel, Icon, Kind, Meta};

/// The texture's file in prepared resources, if it is a texture they hold. Only a name
/// [`Icon::parse`] takes is looked for, and only a file that really is inside the resources'
/// textures is answered: a name with a path in it, or a link leading out, reads nothing.
pub fn icon_texture(icon: &Icon, resources: &Path) -> Option<PathBuf> {
    let Icon::Texture(name) = icon else {
        return None;
    };
    if !sunrise_launcher_domain::is_texture_name(name) {
        return None;
    }
    let textures = resources
        .join("assets/minecraft/textures")
        .canonicalize()
        .ok()?;
    ["block", "item"]
        .iter()
        .filter_map(|kind| {
            textures
                .join(kind)
                .join(format!("{name}.png"))
                .canonicalize()
                .ok()
        })
        .find(|p| p.starts_with(&textures) && p.is_file())
}

/// The installed build on this channel, from the newest-first on-disk list. A test build
/// from a draft's files is chosen only by its exact tag, never as the newest.
pub fn newest_installed(channel: Channel, installed: &[String]) -> Option<&String> {
    installed.iter().find(|tag| {
        Channel::of_build(tag) == Some(channel) && !crate::draft_releases::is_test_build(tag)
    })
}

/// The target of an unpinned profile: stay on an installed build until the player updates.
/// A Stable install keeps working without the catalogue. Before there is one, the profile
/// can play Nightly while there is no downloadable Stable, without changing its channel.
///
/// This reads the launcher's draft test-build mark through [`newest_installed`], so it stays
/// here beside the other instance state rather than in the pure `launcher-domain` crate.
pub fn play_target<'a>(
    channel: Channel,
    releases: &'a [backend::Release],
    installed: &'a [String],
    nightly: Option<&'a String>,
    selected: Option<&'a String>,
) -> Option<&'a String> {
    let latest = channel.builds(releases).next().map(|release| &release.tag);
    let have = match channel {
        Channel::Stable | Channel::Dev => newest_installed(channel, installed),
        Channel::Nightly => nightly,
    };
    match (have, latest) {
        // The newest published build wins a same-day tie once it is actually on disk.
        (Some(have), Some(latest))
            if installed.contains(latest)
                && backend::compare_game_versions(latest, have).is_ge() =>
        {
            Some(latest)
        }
        (Some(have), _) => Some(have),
        (None, Some(latest)) => Some(latest),
        (None, None) if channel == Channel::Stable => {
            play_target(Channel::Nightly, releases, installed, nightly, selected)
        }
        (None, None) if channel == Channel::Nightly => {
            selected.filter(|tag| Channel::of_build(tag).is_none())
        }
        (None, None) => None,
    }
}

impl Instance {
    /// Its variables for the game and the commands around it ([`crate::launch::instance_vars`]).
    pub fn vars(&self) -> crate::launch::Vars {
        crate::launch::instance_vars(&self.name, &self.id, &self.home)
    }
}

/// `instances.json`: each instance's choices, by id.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    instances: BTreeMap<String, Meta>,
    #[serde(flatten)]
    other: BTreeMap<String, serde_json::Value>,
}

impl Store {
    fn load(data: &Path) -> Store {
        Self::read(data).unwrap_or_default()
    }

    fn read(data: &Path) -> Result<Store, String> {
        let path = data.join(FILE);
        let error = |e: std::io::Error| format!("{FILE}: {e}");
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() && !crate::native_cache::linked(&meta) => (),
            Ok(_) => return Err(error(std::io::ErrorKind::InvalidInput.into())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(error(e)),
        }
        let bytes = fs::read(path).map_err(error)?;
        serde_json::from_slice(&bytes).map_err(|e| format!("{FILE}: {e}"))
    }

    fn save(&self, data: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| format!("{FILE}: {e}"))?;
        crate::profile_txn::durable::replace(data, &data.join(FILE), &bytes)
            .map_err(|e| format!("{FILE}: {e}"))
    }

    fn mutate<T>(
        data: &Path,
        change: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        Self::locked(data, |store| {
            let result = change(store)?;
            store.save(data)?;
            Ok(result)
        })
    }

    fn locked<T>(
        data: &Path,
        run: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        Self::under_lock(data, || {
            Self::read(data).and_then(|mut store| run(&mut store))
        })?
    }

    fn under_lock<T>(data: &Path, run: impl FnOnce() -> T) -> Result<T, String> {
        let lock_error = |e: std::io::Error| format!("{FILE}: {e}");
        fs::create_dir_all(data).map_err(lock_error)?;
        let gate = data.join(".instances.lock");
        match fs::symlink_metadata(&gate) {
            Ok(meta) if meta.is_file() && !crate::native_cache::linked(&meta) => (),
            Ok(_) => return Err(lock_error(std::io::ErrorKind::InvalidInput.into())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(lock_error(e)),
        }
        let lock = fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(gate)
            .map_err(lock_error)?;
        lock.lock().map_err(lock_error)?;
        let result = run();
        let _ = lock.unlock();
        Ok(result)
    }
}

/// Filesystem identity observed when the profile was listed, not just its reusable slug.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Incarnation {
    work: Identity,
    home: Identity,
}

impl Incarnation {
    pub(crate) fn of(work: &Path, home: &Path) -> Result<Self, String> {
        let id = Identity::of(work)?;
        Ok(Self {
            work: id,
            home: if work == home {
                id
            } else {
                Identity::of(home)?
            },
        })
    }

    fn at_detachment(self, instance: &Instance, detached: &Path) -> Result<(), String> {
        let inner = instance
            .home
            .strip_prefix(&instance.work)
            .map_err(|_| lifecycle::changed())?;
        let home = if inner.as_os_str().is_empty() {
            detached.to_owned()
        } else {
            detached.join(inner)
        };
        if Self::of(detached, &home).ok() != Some(self) {
            return Err(lifecycle::changed());
        }
        Ok(())
    }

    pub(crate) fn check(self, instance: &Instance, actual: Identity) -> Result<(), String> {
        if actual != self.home
            || (instance.work != instance.home && Identity::of(&instance.work)? != self.work)
        {
            return Err(lifecycle::changed());
        }
        Ok(())
    }
}

/// Write the first profile choice before settings can be saved. An older launcher could have
/// used the implicit Sunrise profile without ever writing `instances.json`.
pub fn initialize_default() -> Result<(), String> {
    initialize_default_in(&backend::game_home(), &backend::data_dir())
}

fn initialize_default_in(game_home: &Path, data: &Path) -> Result<(), String> {
    use std::io::ErrorKind::NotFound;
    let missing = |p: &Path| fs::symlink_metadata(p).is_err_and(|e| e.kind() == NotFound);
    let empty = |p: &Path| match fs::read_dir(p) {
        Ok(mut entries) => entries.next().is_none(),
        Err(e) => e.kind() == NotFound,
    };
    if ![FILE, crate::paths::SETTINGS, "accounts.json"]
        .iter()
        .all(|f| missing(&data.join(f)))
        || !empty(game_home)
        || !empty(&data.join("versions"))
    {
        return Ok(());
    }
    Store::under_lock(data, || {
        use std::io::Write as _;

        let store = Store {
            instances: BTreeMap::from([(DEFAULT.to_owned(), {
                let mut meta = Meta::default();
                meta.channel = Some(Channel::Stable);
                meta
            })]),
            ..Store::default()
        };
        let bytes = serde_json::to_vec_pretty(&store).map_err(|e| format!("{FILE}: {e}"))?;
        let write = || -> std::io::Result<()> {
            let mut part = tempfile::Builder::new()
                .prefix(".instances-")
                .tempfile_in(data)?;
            part.write_all(&bytes)?;
            part.as_file().sync_all()?;
            match part.persist_noclobber(data.join(FILE)) {
                Ok(_) => (),
                Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(e) => return Err(e.error),
            }
            Ok(())
        };
        write().map_err(|e| format!("{FILE}: {e}"))
    })?
}

#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    /// `sunrise`, an own instance's folder name, `port:<pack>` or `pack:<id>`.
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// The folder the pack's tools work in: the game's home for Default and Own instances.
    pub work: PathBuf,
    /// The game folder it plays in: saves, options.txt, packs.
    pub home: PathBuf,
    #[doc(hidden)]
    pub incarnation: Option<Incarnation>,
    pub icon: Option<Icon>,
    pub group: Option<String>,
    /// Its pinned Sunrise version, if any (`Meta::version`).
    pub version: Option<String>,
    /// Which builds it follows when it keeps no version.
    pub channel: Channel,
    /// Its saved native build choice, or Optimized before the player chooses one.
    pub native_profile: crate::native_mods::Profile,
    pub played: Duration,
    pub last_played: Option<SystemTime>,
    /// What runs around the game (`launch::wrap`).
    pub launch: Launch,
}

impl Instance {
    /// Admit this exact profile before a mod-browser operation queues: the folder exists and
    /// its filesystem identity still matches the one listed with the instance.
    pub(crate) fn admit(&self) -> Result<Identity, String> {
        if self.kind == Kind::Default && self.incarnation.is_none() {
            fs::create_dir_all(&self.home).map_err(|e| format!("{}: {e}", self.home.display()))?;
        }
        let identity = Identity::of(&self.home)?;
        match self.incarnation {
            Some(expected) => expected.check(self, identity)?,
            None if self.kind == Kind::Default => (),
            None => return Err(lifecycle::changed()),
        }
        Ok(identity)
    }

    /// Preserve the accepted selection, or stamp the default directory created by first Play.
    pub(crate) fn after_admission(&self, home: Identity) -> Self {
        let mut instance = self.clone();
        instance
            .incarnation
            .get_or_insert(Incarnation { work: home, home });
        instance
    }

    /// The installed version this instance plays: an explicit request or pin wins, then the
    /// newest installed build on its channel, with an offline fallback.
    pub fn resolve_version(&self, requested: Option<&str>) -> Result<String, String> {
        match (requested, &self.version) {
            (None, Some(pinned)) => backend::resolve_version(Some(pinned)).map_err(|_| {
                crate::i18n::trf(
                    "{profile} uses {build}, which is not installed. Install it, or use Switch to choose another build.",
                    &[("profile", &self.name), ("build", pinned)],
                )
            }),
            (Some(tag), _) => backend::resolve_version(Some(tag)),
            (None, None) => {
                let selected = backend::Settings::load().selected;
                let (installed, nightly) = backend::play_installations(selected.as_deref());
                let selected = selected.filter(|tag| installed.contains(tag));
                play_target(
                    self.channel,
                    &[],
                        &installed,
                        nightly.as_ref(),
                        selected.as_ref().or(installed.first()),
                    )
                    .cloned()
                    .ok_or_else(backend::no_version_installed)
            }
        }
    }
}

/// The last second of the year 9999: the latest time a saved file is believed about.
const LATEST: Duration = Duration::from_secs(253_402_300_799);

/// A time read from a saved file (`since_epoch` after 1970), or None for one no calendar holds:
/// seconds that fit a `SystemTime` can still be past any date, and drawing one as a date panics.
/// Read as "never", like a file that names no time.
pub fn calendar_time(since_epoch: Duration) -> Option<SystemTime> {
    (since_epoch <= LATEST)
        .then(|| UNIX_EPOCH.checked_add(since_epoch))
        .flatten()
}

#[cfg(test)]
mod tests;
