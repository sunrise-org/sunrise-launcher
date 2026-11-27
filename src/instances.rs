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

mod deletion_recovery;
pub(crate) mod lifecycle;
use crate::managed_play::{Profile as ManagedProfile, ReferencePublication};
use crate::profile_txn::{durable, Identity, NameGate};
pub use deletion_recovery::{interrupted_deletions, InterruptedDeletion};
use lifecycle::{initial_choices, GATES, INITIAL, RETIRED};

/// The default instance's id: the game's home itself.
pub const DEFAULT: &str = "sunrise";
const FILE: &str = "instances.json";
/// Where new instances live, in the game's home.
const OWN: &str = "instances";
/// Where a new instance is put together, in the game's home (the same disk as [`OWN`], so
/// moving it there is one rename). Never listed: an instance exists once it is whole.
const STAGING: &str = ".new-instance";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The game's home: what every player had before instances.
    Default,
    /// Made in the launcher: `instances/<slug>/`.
    Own,
    /// A modpack from Sunrise's registry.
    Pack,
}

/// What an instance shows in the sidebar: a block or item texture from the player's prepared
/// files (never shipped), or a colour.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Icon {
    /// A texture name: `grass_block` (block/ is looked in first, then item/).
    Texture(String),
    /// `#rrggbb`.
    Colour(String),
}

impl Icon {
    /// `#3fa7d6` is a colour; a texture's name is lowercase letters, digits and `_`.
    pub fn parse(text: &str) -> Result<Icon, String> {
        let text = text.trim();
        if let Some(hex) = text.strip_prefix('#') {
            if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Ok(Icon::Colour(format!("#{}", hex.to_ascii_lowercase())));
            }
        } else if texture_name(text) {
            return Ok(Icon::Texture(text.into()));
        }
        Err(format!(
            "{text}: give a block or item name (grass_block) or a colour (#3fa7d6)"
        ))
    }

    /// The texture's file in prepared resources, if it is a texture they hold. Only a name
    /// [`Icon::parse`] takes is looked for, and only a file that really is inside the resources'
    /// textures is answered: a name with a path in it, or a link leading out, reads nothing.
    pub fn texture(&self, resources: &Path) -> Option<PathBuf> {
        let Icon::Texture(name) = self else {
            return None;
        };
        if !texture_name(name) {
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
}

/// A texture's name: lowercase letters, digits and `_`, so never a path.
fn texture_name(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

// A saved icon is held to what the dialog and the command line take; one that isn't is no icon,
// and never a reason to lose the instance's other choices.
fn saved_icon<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<Icon>, D::Error> {
    let saved: Option<Icon> =
        serde_json::from_value(serde_json::Value::deserialize(deserializer)?).ok();
    Ok(saved.filter(|icon| match icon {
        Icon::Texture(name) => texture_name(name),
        Icon::Colour(hex) => Icon::parse(hex).as_ref() == Ok(icon),
    }))
}

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
        "That file isn't a picture the launcher can read (PNG, JPEG, GIF, WebP).".into()
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

// A malformed preference must not invalidate unrelated instance metadata.
fn saved_profile<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<crate::native_mods::Profile>, D::Error> {
    Ok(serde_json::from_value(serde_json::Value::deserialize(deserializer)?).ok())
}

/// Which builds an instance's "latest" follows. Missing in a saved instance: Nightly, so nobody
/// is moved to an older build.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    #[default]
    Nightly,
    Dev,
}

impl Channel {
    /// Publication channels, not a classifier for arbitrary tags the player has on disk.
    pub fn of_release(release: &backend::Release) -> Option<Self> {
        use sunrise_launcher_release_verify::{Channel as SignedChannel, NIGHTLY};
        match release.nightly.as_deref() {
            Some(NIGHTLY) => Some(Self::Nightly),
            Some(tag)
                if release.prerelease && SignedChannel::of(tag) == Some(SignedChannel::Dev) =>
            {
                Some(Self::Dev)
            }
            None if !release.prerelease && backend::is_game_tag(&release.tag) => Some(Self::Stable),
            _ => None,
        }
    }

    /// Installed builds can be classified even if their releases have been pruned.
    pub fn of_build(tag: &str) -> Option<Self> {
        if crate::nightly::date_of(tag).is_some() {
            Some(Self::Nightly)
        } else if backend::dev_date(tag).is_some() {
            Some(Self::Dev)
        } else if backend::is_game_tag(tag) {
            Some(Self::Stable)
        } else {
            None
        }
    }

    /// The installed build on this channel, from the newest-first on-disk list. A test build
    /// from a draft's files is chosen only by its exact tag, never as the newest.
    pub fn newest_installed(self, installed: &[String]) -> Option<&String> {
        installed.iter().find(|tag| {
            Self::of_build(tag) == Some(self) && !crate::draft_releases::is_test_build(tag)
        })
    }

    /// The downloadable releases on this channel, in catalogue order (newest first).
    pub fn builds(self, releases: &[backend::Release]) -> impl Iterator<Item = &backend::Release> {
        releases
            .iter()
            .filter(move |r| r.asset.is_some() && Self::of_release(r) == Some(self))
    }

    /// The target of an unpinned profile: stay on an installed build until the player updates.
    /// A Stable install keeps working without the catalogue. Before there is one, the profile
    /// can play Nightly while there is no downloadable Stable, without changing its channel.
    pub fn play_target<'a>(
        self,
        releases: &'a [backend::Release],
        installed: &'a [String],
        nightly: Option<&'a String>,
        selected: Option<&'a String>,
    ) -> Option<&'a String> {
        let latest = self.builds(releases).next().map(|release| &release.tag);
        let have = match self {
            Self::Stable | Self::Dev => self.newest_installed(installed),
            Self::Nightly => nightly,
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
            (None, None) if self == Self::Stable => {
                Self::Nightly.play_target(releases, installed, nightly, selected)
            }
            (None, None) if self == Self::Nightly => {
                selected.filter(|tag| Self::of_build(tag).is_none())
            }
            (None, None) => None,
        }
    }

    /// What an unpinned profile follows for now. Without a Stable download it plays Nightly
    /// without changing the saved channel; Stable takes over when one becomes available.
    pub fn following(self, releases: &[backend::Release]) -> Self {
        if self == Self::Stable && self.builds(releases).next().is_none() {
            Self::Nightly
        } else {
            self
        }
    }
}

// An unknown channel must not invalidate unrelated instance metadata.
fn saved_channel<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Channel>, D::Error> {
    Ok(serde_json::from_value(serde_json::Value::deserialize(deserializer)?).ok())
}

fn saved_choice_id<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<[u8; 16]>, D::Error> {
    Ok(serde_json::from_value(serde_json::Value::deserialize(deserializer)?).ok())
}

/// The player's choices about one instance.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(
        default,
        deserialize_with = "saved_icon",
        skip_serializing_if = "Option::is_none"
    )]
    pub icon: Option<Icon>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// The Sunrise version this instance plays, kept when newer ones are installed. None: the
    /// launcher's own version, which follows updates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The builds "latest" means for it; missing is [`Channel::Nightly`].
    #[serde(
        default,
        deserialize_with = "saved_channel",
        skip_serializing_if = "Option::is_none"
    )]
    pub channel: Option<Channel>,
    /// The last committed switch build choice already reflected (or superseded) here.
    #[serde(
        default,
        deserialize_with = "saved_choice_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) build_choice_applied: Option<[u8; 16]>,
    /// This instance's saved native build choice; missing or unknown starts at Optimized.
    #[serde(
        default,
        deserialize_with = "saved_profile",
        skip_serializing_if = "Option::is_none"
    )]
    pub native_profile: Option<crate::native_mods::Profile>,
    /// Seconds played in it, every run started by the launcher (the window or `play`) added up.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub played_secs: u64,
    /// When its last run ended, in seconds since 1970.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_played: Option<u64>,
    /// What runs around the game: a pre-launch command, a wrapper, variables (`launch`).
    #[serde(default, skip_serializing_if = "Launch::is_empty")]
    pub launch: Launch,
    /// Set by an earlier launcher for a Minecraft: Java Edition instance. This launcher no
    /// longer starts those: such an instance is not listed, and its record is kept exactly as it
    /// was, whatever it holds, whenever `instances.json` is saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java: Option<serde_json::Value>,
    /// The same earlier launcher's put-aside Java settings of an instance that played Sunrise
    /// meanwhile: kept as it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java_kept: Option<serde_json::Value>,
    #[serde(flatten)]
    other: BTreeMap<String, serde_json::Value>,
}

impl Instance {
    /// Its variables for the game and the commands around it ([`crate::launch::instance_vars`]).
    pub fn vars(&self) -> crate::launch::Vars {
        crate::launch::instance_vars(&self.name, &self.id, &self.home)
    }
}

fn is_zero(n: &u64) -> bool {
    *n == 0
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
    if ![FILE, "settings.json", "accounts.json"]
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
            instances: BTreeMap::from([(
                DEFAULT.to_owned(),
                Meta {
                    channel: Some(Channel::Stable),
                    ..Meta::default()
                },
            )]),
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
                self.channel
                    .play_target(
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

/// The instances, by the default game home and the launcher's data folder: Sunrise first, then
/// by group and name.
pub fn list() -> Vec<Instance> {
    let out = list_in(&backend::game_home(), &backend::data_dir());
    register_profile_names(&out);
    out
}

/// Read the atomic metadata snapshot without housekeeping or taking a profile gate.
pub(crate) fn list_for_storage() -> Result<Vec<Instance>, String> {
    Ok(list_for_storage_until(&crate::storage::Cancel::default())?
        .expect("uncancellable storage profile snapshot"))
}

/// Like [`list_for_storage`], stopping early when `cancel` is set during profile or pack discovery.
pub(crate) fn list_for_storage_until(
    cancel: &crate::storage::Cancel,
) -> Result<Option<Vec<Instance>>, String> {
    if cancel.is_cancelled() {
        return Ok(None);
    }
    let data = backend::data_dir();
    let store = Store::read(&data)?;
    let out = list_snapshot_until(&backend::game_home(), &data, store, true, false, &|| {
        cancel.is_cancelled()
    });
    if let Some(instances) = &out {
        register_profile_names(instances);
    }
    Ok(out)
}

/// Read the profile directories without the migrations or housekeeping of `list`.
pub(crate) fn profile_homes_strict(game: &Path) -> Result<Vec<PathBuf>, String> {
    let mut homes = Vec::new();
    for (directory, pack) in [(OWN, false), ("sunrise-packs", true)] {
        let root = game.join(directory);
        let error = |e: std::io::Error| format!("{}: {e}", root.display());
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => match fs::symlink_metadata(&root)
            {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                _ => return Err(error(e)),
            },
            Err(e) => return Err(error(e)),
        };
        let directory = |path: &Path, meta: fs::Metadata| -> Result<bool, String> {
            if sunrise_launcher_smod::runtime_leases::linked(&meta) {
                // Skip links only when their target is known not to be a directory.
                return Ok(fs::metadata(path).map_err(error)?.is_dir());
            }
            Ok(meta.is_dir())
        };
        for entry in entries {
            let path = entry.map_err(error)?.path();
            if !directory(&path, fs::symlink_metadata(&path).map_err(error)?)? {
                continue;
            }
            if pack {
                let home = registry_pack::home(&path);
                match fs::symlink_metadata(&home) {
                    Ok(meta) => {
                        if directory(&home, meta)? {
                            homes.push(home);
                        }
                    }
                    // The work folder below already covers a not-yet-created home.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                    Err(e) => return Err(error(e)),
                }
            }
            homes.push(path);
        }
    }
    Ok(homes)
}

fn register_profile(instance: &Instance) {
    if !(instance.kind == Kind::Default && instance.name.eq_ignore_ascii_case("Sunrise")) {
        crate::launcher_log::hide_profile_name(&instance.name);
    }
    if instance.kind == Kind::Own {
        crate::launcher_log::hide_profile_name(&instance.id);
    }
}

fn register_profile_names(instances: &[Instance]) {
    for instance in instances {
        register_profile(instance);
    }
}

fn list_in(game_home: &Path, data: &Path) -> Vec<Instance> {
    list_with_snapshot(game_home, data, true, || {})
}

fn list_with_snapshot(
    game_home: &Path,
    data: &Path,
    housekeeping: bool,
    after_read: impl FnOnce(),
) -> Vec<Instance> {
    match Store::under_lock(data, || {
        let stored = Store::read(data);
        after_read();
        match stored {
            Ok(store) => list_snapshot(game_home, data, store, true, housekeeping),
            Err(_) => list_snapshot(game_home, data, Store::default(), false, false),
        }
    }) {
        Ok(instances) => instances,
        // Keep the listing usable, but don't authorize mutation with unpaired choices.
        Err(_) => list_snapshot(game_home, data, Store::load(data), false, false),
    }
}

fn list_snapshot(
    game_home: &Path,
    data: &Path,
    store: Store,
    admitted: bool,
    housekeeping: bool,
) -> Vec<Instance> {
    list_snapshot_until(game_home, data, store, admitted, housekeeping, &|| false)
        .expect("uncancellable profile snapshot")
}

fn list_snapshot_until(
    game_home: &Path,
    data: &Path,
    mut store: Store,
    admitted: bool,
    housekeeping: bool,
    cancelled: &dyn Fn() -> bool,
) -> Option<Vec<Instance>> {
    if cancelled() {
        return None;
    }
    let mut found = vec![(
        DEFAULT.to_string(),
        "Sunrise".to_string(),
        Kind::Default,
        game_home.to_path_buf(),
        game_home.to_path_buf(),
    )];
    let mut own = Vec::new();
    if let Ok(entries) = fs::read_dir(game_home.join(OWN)) {
        for entry in entries {
            if cancelled() {
                return None;
            }
            let Ok(entry) = entry else {
                continue;
            };
            if entry.path().is_dir() {
                if let Ok(name) = entry.file_name().into_string() {
                    own.push(name);
                }
            }
        }
    }
    own.sort();
    // Directory renames and metadata snapshots share the lock. Play/Delete need the initial
    // choices even after an interrupted create, but a refused request must not save the store.
    if admitted {
        let parent = game_home.join(OWN);
        let before = store.instances.len();
        if housekeeping && parent.is_dir() {
            store.instances.retain(|id, _| {
                id == DEFAULT
                    || crate::browser::safe_name(id) != Some(id)
                    || !fs::symlink_metadata(parent.join(id))
                        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            });
        }
        let retained = store.instances.len();
        for id in &own {
            if cancelled() {
                return None;
            }
            if !store.instances.contains_key(id) {
                if let Some(meta) = initial_choices(&parent.join(id)) {
                    store.instances.insert(id.clone(), meta);
                }
            }
        }
        if housekeeping && (retained != before || store.instances.len() != retained) {
            if let Err(error) = store.save(data) {
                crate::elog!("Sunrise: instances.json is not saved: {error}");
            }
        }
    }
    // An instance an earlier launcher made for Minecraft: Java Edition is not listed: its folder
    // and its record in `instances.json` stay as they are.
    own.retain(|slug| store.instances.get(slug).is_none_or(|m| m.java.is_none()));
    for slug in own {
        let dir = game_home.join(OWN).join(&slug);
        found.push((slug.clone(), slug, Kind::Own, dir.clone(), dir));
    }
    for p in registry_pack::list_in_until(&game_home.join("sunrise-packs"), cancelled)? {
        let home = registry_pack::home(&p.work);
        found.push((format!("pack:{}", p.id), p.name, Kind::Pack, p.work, home));
    }
    let mut recovered_choices = false;
    for (id, _, kind, _, home) in &found {
        if cancelled() {
            return None;
        }
        if *kind == Kind::Pack {
            continue;
        }
        let Some(choice) = pending_build_choice(data, home) else {
            continue;
        };
        if store
            .instances
            .get(id)
            .is_some_and(|meta| meta.build_choice_applied == Some(choice.id))
        {
            continue;
        }
        let meta = store
            .instances
            .entry(id.clone())
            .or_insert_with(|| initial_choices(home).unwrap_or_default());
        if apply_committed_choice(meta, &choice) {
            recovered_choices = true;
        }
    }
    if admitted && housekeeping && recovered_choices {
        if let Err(error) = store.save(data) {
            crate::elog!("Sunrise: instances.json is not saved: {error}");
        }
    }
    let mut out = Vec::with_capacity(found.len());
    for (id, name, kind, work, home) in found {
        if cancelled() {
            return None;
        }
        let meta = store.instances.get(&id).cloned().unwrap_or_default();
        out.push(Instance {
            name: meta.name.unwrap_or(name),
            icon: meta.icon,
            group: meta.group,
            version: meta.version,
            channel: meta.channel.unwrap_or_default(),
            native_profile: meta
                .native_profile
                .unwrap_or(crate::native_mods::Profile::Optimized),
            played: Duration::from_secs(meta.played_secs),
            last_played: meta
                .last_played
                .and_then(|s| calendar_time(Duration::from_secs(s))),
            launch: meta.launch,
            id,
            kind,
            incarnation: admitted
                .then(|| Incarnation::of(&work, &home).ok())
                .flatten(),
            work,
            home,
        });
    }
    out[1..].sort_by_key(|i| (i.group.clone(), i.name.to_lowercase()));
    Some(out)
}

/// The instance an id, a name (any case) or a pack's folder name names.
pub fn find(query: &str) -> Result<Instance, String> {
    find_in(&list(), query)
}

fn find_in(all: &[Instance], query: &str) -> Result<Instance, String> {
    let q = query.trim();
    let bare = |id: &str| id.split_once(':').map_or(id, |(_, rest)| rest).to_string();
    all.iter()
        .find(|i| i.id == q)
        .or_else(|| all.iter().find(|i| i.name.eq_ignore_ascii_case(q)))
        .or_else(|| all.iter().find(|i| bare(&i.id) == q))
        .cloned()
        .ok_or_else(|| format!("No profile {q}: sunrise-launcher instance list shows them"))
}

/// Moves an instance's folder to the Trash (where the player can get it back, worlds and all)
/// and forgets it: its choices, and the selection when Play started it. Sunrise's own instance,
/// the game's home, stays.
pub fn delete(query: &str) -> Result<Instance, String> {
    let (gone, saved) = delete_with_selection(query)?;
    saved?;
    Ok(gone)
}

/// A completed deletion stays completed even if saving the new selection fails.
pub fn delete_with_selection(query: &str) -> Result<(Instance, Result<(), String>), String> {
    let game_home = backend::game_home();
    let data = backend::data_dir();
    register_profile_names(&list_with_snapshot(&game_home, &data, false, || {}));
    let gone = delete_in(&game_home, &data, query, &crate::desktop_seams::to_trash)
        .map_err(|error| error.to_string())?;
    let mut settings = backend::Settings::load();
    let saved = if clear_deleted_selection(&mut settings, &gone.id) {
        settings.save()
    } else {
        Ok(())
    };
    Ok((gone, saved))
}

/// Takes the same fallback as the command, without overwriting other live window settings.
pub fn clear_deleted_selection(settings: &mut backend::Settings, id: &str) -> bool {
    settings
        .instance
        .take_if(|selected| selected == id)
        .is_some()
}

/// Move the exact folder shown in the window's confirmation, not a fresh lookup by slug.
/// The caller saves its selection once the move settles.
pub fn trash_confirmed(instance: Instance) -> Result<Instance, DeleteError> {
    register_profile(&instance);
    delete_selected(
        &backend::game_home(),
        &backend::data_dir(),
        instance,
        &crate::desktop_seams::to_trash,
    )
}

#[derive(Debug)]
pub struct DeleteError {
    pub detail: String,
    /// Set when the folder left its original path but could not reach the Trash.
    pub retained: Option<InterruptedDeletion>,
}

impl From<String> for DeleteError {
    fn from(detail: String) -> Self {
        Self {
            detail,
            retained: None,
        }
    }
}

impl std::fmt::Display for DeleteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(path) = &self.retained {
            let folder = path.folder().to_string_lossy();
            write!(
                f,
                "{}: {}",
                crate::i18n::trf("The profile was kept in {folder}", &[("folder", &folder)]),
                self.detail
            )
        } else {
            f.write_str(&self.detail)
        }
    }
}

fn delete_in(
    game_home: &Path,
    data: &Path,
    query: &str,
    trash: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<Instance, DeleteError> {
    let instance = find_in(&list_with_snapshot(game_home, data, false, || {}), query)?;
    delete_selected(game_home, data, instance, trash)
}

fn delete_selected(
    game_home: &Path,
    data: &Path,
    instance: Instance,
    trash: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<Instance, DeleteError> {
    if instance.kind == Kind::Pack {
        return Err(registry_pack::remove_unavailable().into());
    }
    if instance.kind == Kind::Default {
        return Err(crate::i18n::trf(
            "{name} is the game's own folder: it can't be deleted.",
            &[("name", &instance.name)],
        )
        .into());
    }
    delete_selected_at(
        game_home,
        data,
        instance,
        durable::move_new_directory,
        trash,
    )
}

fn delete_selected_at(
    game_home: &Path,
    data: &Path,
    instance: Instance,
    detach: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
    trash: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<Instance, DeleteError> {
    let work = &instance.work;
    let expected = instance.incarnation.ok_or_else(lifecycle::changed)?;
    let profile =
        ManagedProfile::open_instance(game_home, data, &instance).map_err(String::from)?;
    let problem = |error: &dyn std::fmt::Display| {
        crate::i18n::trf(
            "{name} could not go to the Trash: {error}",
            &[("name", &instance.name), ("error", &error.to_string())],
        )
    };
    let root = game_home.canonicalize().map_err(|e| problem(&e))?;
    let retired = creation_parent(&root, RETIRED).map_err(|e| problem(&e))?;
    let private = tempfile::tempdir_in(&retired).map_err(|e| problem(&e))?;
    let gone = private
        .path()
        .join(work.file_name().ok_or_else(lifecycle::changed)?);
    let recovery = InterruptedDeletion::record(&retired, gone.clone(), instance.id.clone())
        .map_err(|e| problem(&e))?;
    // From this point neither an ancillary error nor a destructor may empty the detached
    // folder. If Trash is unavailable, preserve the whole profile in the recovery folder.
    let _ = private.keep();
    let detached = (|| {
        let _references = ReferencePublication::begin_at(data, game_home, profile.home())?;
        Store::mutate(data, |store| {
            detach(work, &gone).map_err(|e| problem(&e))?;
            if let Err(changed) = expected.at_detachment(&instance, &gone) {
                // The name still belongs to this operation, but an external writer may
                // have filled it. Never restore by overwriting that directory.
                return match durable::move_new_directory(&gone, work) {
                    Ok(()) => Err(changed),
                    Err(error) => Err(crate::i18n::trf(
                        "The profile changed and could not be restored: {error}.",
                        &[("error", &error.to_string())],
                    )),
                };
            }
            store.instances.remove(&instance.id);
            Ok(())
        })
    })();
    let trashed = detached.and_then(|()| trash(&gone));
    let retained = recovery.retained();
    trashed.map_err(|detail| DeleteError { retained, detail })?;
    Ok(instance)
}

/// Resolve headless Play without repairing unrelated profile metadata on a refused request.
pub fn for_play(query: Option<&str>) -> Result<Instance, String> {
    let all = list_with_snapshot(&backend::game_home(), &backend::data_dir(), false, || {});
    register_profile_names(&all);
    let found = match query {
        Some(name) => find_in(&all, name),
        None => selection_in(&all, backend::Settings::load().instance.as_deref()),
    }?;
    if found.kind == Kind::Pack {
        return Err(registry_pack::play_unavailable());
    }
    Ok(found)
}

/// The instance Play starts: the one picked in the launcher, else Sunrise.
pub fn selected() -> Instance {
    let all = list();
    backend::Settings::load()
        .instance
        .and_then(|id| all.iter().find(|i| i.id == id).cloned())
        .unwrap_or_else(|| all[0].clone())
}

/// The saved selection when it names an instance that isn't listed any more: a pack an earlier
/// launcher ported, or a setup it made for another game. Its folder is still there, untouched,
/// but nothing here can use it.
pub fn retired_selection(all: &[Instance], saved: Option<&str>) -> Option<String> {
    let id = saved?;
    (!all.iter().any(|i| i.id == id)).then(|| id.to_string())
}

fn retired_message(id: &str) -> String {
    let name = id.split_once(':').map_or(id, |(_, rest)| rest);
    crate::i18n::trf(
        "The selected profile {name} was made by an earlier launcher and isn't supported any more. Its files are untouched. Choose another: sunrise-launcher instance select <name>",
        &[("name", name)],
    )
}

/// The message for a retired selection ([`retired_selection`]).
pub fn retired_selection_message(id: &str) -> String {
    retired_message(id)
}

/// The instance a command without `--instance` acts on: [`selected`], except that a retired
/// selection is an error naming it, so nothing is written to Sunrise's setup in its place.
pub fn read_profile(query: Option<&str>) -> Result<Instance, String> {
    let all = list_with_snapshot(&backend::game_home(), &backend::data_dir(), false, || {});
    register_profile_names(&all);
    match query {
        Some(name) => find_in(&all, name),
        None => selection_in(&all, backend::Settings::load().instance.as_deref()),
    }
}

pub fn selection() -> Result<Instance, String> {
    selection_in(&list(), backend::Settings::load().instance.as_deref())
}

fn selection_in(all: &[Instance], saved: Option<&str>) -> Result<Instance, String> {
    if let Some(id) = retired_selection(all, saved) {
        return Err(retired_message(&id));
    }
    Ok(saved
        .and_then(|id| all.iter().find(|i| i.id == id).cloned())
        .unwrap_or_else(|| all[0].clone()))
}

pub fn select(query: &str) -> Result<Instance, String> {
    let instance = find(query)?;
    let mut settings = backend::Settings::load();
    settings.instance = (instance.kind != Kind::Default).then(|| instance.id.clone());
    settings.save()?;
    Ok(instance)
}

/// An instance's folder name: its name in lowercase words joined by dashes.
fn slug(name: &str) -> String {
    let words: Vec<String> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    if words.is_empty() {
        "pack".into()
    } else {
        words.join("-")
    }
}

/// What a copy of a game folder leaves out: other instances and packs, logs and crash reports,
/// caches, data collection's state, and sign-in sessions (a session never leaves its folder).
const NOT_COPIED: &[&str] = &[
    OWN,
    STAGING,
    "ports",
    "sunrise-packs",
    "logs",
    "crash-reports",
    "sessions",
    "mod-browser",
    ".mod-browser.lock",
    ".install-txn.lock",
    GATES,
    INITIAL,
    RETIRED,
    ".sunrise-cache",
    // Far terrain's copies of what servers sent, rebuilt as the instance plays.
    "server-terrain",
];

/// Files of the player, not of one instance, that a new instance starts with: the keys and video
/// settings, and the offline name (so the player stays the same player in its worlds).
const PLAYER_FILES: &[&str] = &["options.txt", "sunrise-player.txt"];

/// Creates an instance named `name`: empty but for the player's options and offline name, or a
/// copy of `from`'s game folder (its worlds, options and packs). Nothing of `from` changes.
pub fn create(name: &str, from: Option<&str>) -> Result<Instance, String> {
    crate::launcher_log::hide_profile_name(name);
    let from = from.map(find).transpose()?;
    let created = create_whole_in(
        &backend::game_home(),
        &backend::data_dir(),
        name,
        from.as_ref(),
        None,
        |_| {},
    )?;
    register_profile(&created);
    Ok(created)
}

/// The directory and build the creation picker told the player it would duplicate.
#[derive(Clone)]
pub struct CopyPreview {
    instance: Instance,
    base: Option<String>,
}

impl CopyPreview {
    /// Read only; the worker rechecks both the directory and this build under admission.
    pub fn read(instance: &Instance) -> Result<Self, String> {
        let refs = crate::profile_txn::references(&instance.home)?;
        let base = refs
            .pointer
            .as_deref()
            .map(crate::managed_play::Active::parse)
            .transpose()?
            .map(|active| active.base().to_owned());
        if let Some(expected) = instance.incarnation {
            expected.check(instance, Identity::of(&instance.home)?)?;
        }
        Ok(Self {
            instance: instance.clone(),
            base,
        })
    }

    pub fn base(&self) -> Option<&str> {
        self.base.as_deref()
    }
}

#[derive(Clone, Copy)]
enum CopyFrom<'a> {
    Selected(&'a Instance),
    Preview(&'a CopyPreview),
}

impl<'a> CopyFrom<'a> {
    fn instance(self) -> &'a Instance {
        match self {
            Self::Selected(selected) => selected,
            Self::Preview(preview) => &preview.instance,
        }
    }
}

/// [`create`], with everything else the New Profile dialog chose, as one step: `picture` (None
/// keeps what a copy brought, `Some(None)` leaves it without one) and `choose`, which sets the
/// rest of its choices. An instance appears only once its folder, picture and choices are all in
/// place. Once visible, its initial choices can be recovered even if the launcher exits.
pub fn create_with(
    name: &str,
    from: Option<&CopyPreview>,
    picture: Option<Option<&[u8]>>,
    choose: impl FnOnce(&mut Meta),
) -> Result<Instance, String> {
    crate::launcher_log::hide_profile_name(name);
    if let Some(from) = from {
        register_profile(&from.instance);
    }
    let created = create_with_publish(
        &backend::game_home(),
        &backend::data_dir(),
        name,
        from.map(CopyFrom::Preview),
        picture,
        choose,
        |_| {},
    )?;
    register_profile(&created);
    Ok(created)
}

/// What creating or renaming an instance says when another already has `name`.
pub fn name_taken(name: &str) -> String {
    crate::i18n::trf(
        "There is already a profile called {name}. Choose another name.",
        &[("name", name)],
    )
}

fn create_whole_in(
    game_home: &Path,
    data: &Path,
    name: &str,
    from: Option<&Instance>,
    picture: Option<Option<&[u8]>>,
    choose: impl FnOnce(&mut Meta),
) -> Result<Instance, String> {
    create_with_publish(
        game_home,
        data,
        name,
        from.map(CopyFrom::Selected),
        picture,
        choose,
        |_| {},
    )
}

fn create_with_publish(
    game_home: &Path,
    data: &Path,
    name: &str,
    copy: Option<CopyFrom<'_>>,
    picture: Option<Option<&[u8]>>,
    choose: impl FnOnce(&mut Meta),
    after_visible: impl FnOnce(&Path),
) -> Result<Instance, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A profile needs a name.".into());
    }
    let from = copy.map(CopyFrom::instance);
    if let Some(from) = from.filter(|f| f.kind == Kind::Pack) {
        return Err(format!(
            "{} plays its pack's mods: import the pack again for a second copy.",
            from.name
        ));
    }
    let create_error = |e: std::io::Error| {
        crate::i18n::trf(
            "Could not make profile {name}: {error}",
            &[("name", name), ("error", &e.to_string())],
        )
    };
    fs::create_dir_all(game_home).map_err(create_error)?;
    let root = game_home.canonicalize().map_err(create_error)?;
    let own = creation_parent(&root, OWN).map_err(create_error)?;
    let staging = creation_parent(&root, STAGING).map_err(create_error)?;
    if list_in(game_home, data)
        .iter()
        .any(|i| i.name.eq_ignore_ascii_case(name))
    {
        return Err(name_taken(name));
    }
    let base = slug(name);
    let taken = |s: &str| s == DEFAULT || own.join(s).symlink_metadata().is_ok();
    let slug = (1..)
        .map(|n| {
            if n == 1 {
                base.clone()
            } else {
                format!("{base}-{n}")
            }
        })
        .find(|s| !taken(s))
        .unwrap();
    let dir = own.join(&slug);
    // Each attempt owns one private directory. Move its child, not the temp directory's
    // own path, so dropping the guard can never clean up a reused path after publication.
    Store::read(data)?;
    let stage = tempfile::tempdir_in(&staging).map_err(create_error)?;
    let contents = stage.path().join("home");
    let source = from
        .map(|from| ManagedProfile::open_instance(game_home, data, from).map_err(String::from))
        .transpose()?;
    let active = if let Some(source) = &source {
        let _inputs = source.lock_inputs()?;
        let active = source.active()?;
        if let Some(CopyFrom::Preview(preview)) = copy {
            if active.as_ref().map(crate::managed_play::Active::base) != preview.base() {
                return Err(lifecycle::changed());
            }
        }
        let worlds = crate::worlds::SavedWorlds::claim(&source.home().join("saves"))?;
        copy_home(source.home(), &contents, &worlds).map_err(create_error)?;
        active
            .map(|active| source.stage_copy(active, &contents))
            .transpose()?
    } else {
        fs::create_dir(&contents).map_err(create_error)?;
        for f in PLAYER_FILES {
            let source = game_home.join(f);
            if source.is_file() {
                fs::copy(source, contents.join(f)).map_err(create_error)?;
            }
        }
        None
    };
    let icon = contents.join(PICTURE);
    match picture {
        Some(Some(png)) => fs::write(icon, png).map_err(create_error)?,
        Some(None) if icon.is_file() => fs::remove_file(icon).map_err(create_error)?,
        _ => {}
    }
    let mut choices = Meta {
        name: (name != slug).then(|| name.to_string()),
        icon: from.and_then(|f| f.icon.clone()),
        group: from.and_then(|f| f.group.clone()),
        channel: Some(Channel::Stable),
        ..Meta::default()
    };
    choose(&mut choices);
    if let Some(active) = &active {
        choices.version = Some(active.base().to_owned());
    }
    let bytes = serde_json::to_vec(&choices).map_err(|e| e.to_string())?;
    fs::write(contents.join(INITIAL), bytes).map_err(create_error)?;
    durable::flush_tree(&contents).map_err(create_error)?;
    let _name = NameGate::open(&lifecycle::gate_path(&root, OWN, slug.as_ref())?)?;
    if list_in(game_home, data)
        .iter()
        .any(|i| i.name.eq_ignore_ascii_case(name))
    {
        return Err(name_taken(name));
    }
    match fs::symlink_metadata(&dir) {
        Ok(_) => return Err(name_taken(name)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(create_error(e)),
    }
    {
        let _references = ReferencePublication::begin_at(data, game_home, &root)?;
        if let Some(active) = &active {
            active.verify_artifacts()?;
        }
        crate::run_leases::publish_at(&own, data).map_err(crate::native_mods::cache_error)?;
        crate::run_leases::publish_at(&contents, data).map_err(crate::native_mods::cache_error)?;
        creation_parent(&root, OWN).map_err(create_error)?;
        Store::locked(data, |store| {
            // Clear the old incarnation's row before the new folder can be observed, and
            // don't let another metadata writer fill that slot before publication.
            if store.instances.remove(&slug).is_some() {
                store.save(data)?;
            }
            durable::move_new_directory(&contents, &dir).map_err(create_error)?;
            after_visible(&dir);
            store.instances.insert(slug.clone(), choices);
            match store.save(data) {
                Ok(()) => {
                    let _ = fs::remove_file(dir.join(INITIAL));
                }
                Err(error) => {
                    // The sidecar is already durable. Do not unpublish or discard it.
                    crate::elog!("Sunrise: instances.json is not saved: {error}");
                }
            }
            Ok(())
        })?;
    }
    find_in(&list_in(game_home, data), &slug)
}

/// A real child directory of the canonical game root, not a symlink or a junction.
fn creation_parent(root: &Path, name: &str) -> std::io::Result<PathBuf> {
    let path = root.join(name);
    match fs::create_dir(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let meta = path.symlink_metadata()?;
    if !meta.is_dir() || crate::native_cache::linked(&meta) || path.canonicalize()? != path {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "{} is not a directory inside the game folder",
                path.display()
            ),
        ));
    }
    Ok(path)
}

/// Publish atomically without replacing another attempt's folder, even if it is empty.
#[cfg(not(windows))]
pub(crate) fn publish_new_folder(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt as _;
        let from = CString::new(from.as_os_str().as_bytes())?;
        let to = CString::new(to.as_os_str().as_bytes())?;
        // SAFETY: both pointers name owned, null-terminated path strings for the call.
        let result = unsafe {
            #[cfg(target_os = "linux")]
            let result = libc::renameat2(
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            );
            #[cfg(target_os = "macos")]
            let result = libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL);
            result
        };
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (from, to);
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

fn copy_home(from: &Path, to: &Path, worlds: &crate::worlds::SavedWorlds) -> std::io::Result<()> {
    let saves = from.join("saves");
    let mut pending = vec![(from.to_owned(), to.to_owned())];
    while let Some((source, target)) = pending.pop() {
        if source.parent() == Some(&saves) {
            worlds.verify(&source)?;
        }
        fs::create_dir(&target)?;
        for entry in fs::read_dir(&source)? {
            let entry = entry?;
            let name = entry.file_name();
            if (source == from && NOT_COPIED.iter().any(|n| name == *n))
                || (source.parent() == Some(&saves) && name == crate::worlds::LOCK)
            {
                continue;
            }
            let path = entry.path();
            let meta = path.symlink_metadata()?;
            if crate::native_cache::linked(&meta) {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
            if meta.is_dir() {
                pending.push((path, target.join(name)));
            } else if meta.is_file() {
                fs::copy(path, target.join(name))?;
            } else {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
        }
    }
    // Preserve chosen-versions.json pins while keeping caches inside mod-browser/ out of copies.
    let chosen_dir = from.join("mod-browser");
    if let Ok(dir_meta) = chosen_dir.symlink_metadata() {
        if dir_meta.is_dir() && !crate::native_cache::linked(&dir_meta) {
            let chosen = chosen_dir.join("chosen-versions.json");
            match chosen.symlink_metadata() {
                Ok(meta) if meta.is_file() && !crate::native_cache::linked(&meta) => {
                    let target_dir = to.join("mod-browser");
                    fs::create_dir_all(&target_dir)?;
                    fs::copy(&chosen, target_dir.join("chosen-versions.json"))?;
                }
                Ok(_) => return Err(std::io::ErrorKind::InvalidData.into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
    }
    worlds.verify_all()
}

fn apply_committed_choice(
    meta: &mut Meta,
    choice: &crate::managed_play::CommittedBuildChoice,
) -> bool {
    if meta.build_choice_applied == Some(choice.id) {
        return false;
    }
    meta.version = choice.version.clone();
    if let Some(channel) = choice.channel {
        meta.channel = Some(channel);
    }
    meta.build_choice_applied = Some(choice.id);
    true
}

fn pending_build_choice(
    data: &Path,
    home: &Path,
) -> Option<crate::managed_play::CommittedBuildChoice> {
    crate::managed_play::try_display_active_at(data, home)
        .ok()
        .flatten()
        .and_then(|active| active.build_choice().cloned())
}

/// Persists a switch's already-committed channel or pin choice into `instances.json`.
pub(crate) fn sync_build_choice(
    id: &str,
    home: &Path,
    choice: &crate::managed_play::CommittedBuildChoice,
) -> Result<(), String> {
    Store::mutate(&backend::data_dir(), |store| {
        let meta = store
            .instances
            .entry(id.to_owned())
            .or_insert_with(|| initial_choices(home).unwrap_or_default());
        apply_committed_choice(meta, choice);
        Ok(())
    })
}

/// Changes what the player chose about an instance: its name, icon or group. No folder moves.
pub fn edit(query: &str, change: impl FnOnce(&mut Meta)) -> Result<Instance, String> {
    let game_home = backend::game_home();
    let data = backend::data_dir();
    register_profile_names(&list_in(&game_home, &data));
    let mut proposed_name = None;
    let result = edit_in(&game_home, &data, query, |meta| {
        change(meta);
        proposed_name = meta.name.clone();
    });
    if let Some(name) = proposed_name {
        crate::launcher_log::hide_profile_name(&name);
    }
    if let Ok(instance) = &result {
        register_profile(instance);
    }
    result
}

fn edit_in(
    game_home: &Path,
    data: &Path,
    query: &str,
    change: impl FnOnce(&mut Meta),
) -> Result<Instance, String> {
    let all = list_in(game_home, data);
    let instance = find_in(&all, query)?;
    Store::mutate(data, |store| {
        let meta = store
            .instances
            .entry(instance.id.clone())
            .or_insert_with(|| initial_choices(&instance.home).unwrap_or_default());
        if instance.kind != Kind::Pack {
            if let Some(choice) = pending_build_choice(data, &instance.home) {
                apply_committed_choice(meta, &choice);
            }
        }
        change(meta);
        meta.name = meta.name.take().map(|n| n.trim().to_string());
        meta.group = meta
            .group
            .take()
            .map(|g| g.trim().to_string())
            .filter(|g| !g.is_empty());
        if let Some(name) = meta.name.clone() {
            if name.is_empty() {
                return Err(crate::i18n::tr("A profile needs a name.").into());
            }
            // Only a new name is checked: an import can share a name, and changing anything
            // else about it must still work.
            let renamed = !name.eq_ignore_ascii_case(&instance.name);
            if renamed
                && all.iter().any(|i| {
                    i.id != instance.id
                        && store
                            .instances
                            .get(&i.id)
                            .and_then(|m| m.name.as_deref())
                            .unwrap_or(&i.name)
                            .eq_ignore_ascii_case(&name)
                })
            {
                return Err(name_taken(&name));
            }
        }
        Ok(())
    })?;
    find_in(&list_in(game_home, data), &instance.id)
}

/// Adds a finished run of the game to the instance's play time (by id: a renamed instance keeps
/// it).
/// Complete the accepted run, not the profile (if any) currently using its name.
/// The callback must run synchronously so the profile cannot be replaced during the hook.
pub fn finish_play(
    instance: &Instance,
    played: Duration,
    hook: impl FnOnce(),
) -> Result<(), String> {
    finish_play_in(
        &backend::game_home(),
        &backend::data_dir(),
        instance,
        played,
        hook,
    )
}

fn finish_play_in(
    game: &Path,
    data: &Path,
    instance: &Instance,
    played: Duration,
    hook: impl FnOnce(),
) -> Result<(), String> {
    let expected = instance.incarnation.ok_or_else(lifecycle::changed)?;
    let owner = lifecycle::owner_file(game, &instance.home)?;
    let profile =
        crate::profile_txn::Admission::claim_after_exit(&instance.home, owner.as_deref())?;
    expected.check(instance, profile.identity())?;
    let saved = record_play_in(data, &instance.id, played, SystemTime::now());
    hook();
    saved
}

fn record_play_in(
    data: &Path,
    id: &str,
    played: Duration,
    ended: SystemTime,
) -> Result<(), String> {
    Store::mutate(data, |store| {
        let meta = store.instances.entry(id.to_string()).or_default();
        meta.played_secs = meta.played_secs.saturating_add(played.as_secs());
        meta.last_played = ended.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs());
        Ok(())
    })
}

/// "12 min", "3 h 05 min", "140 h": the play time as players read it.
pub fn play_time(played: Duration) -> String {
    let minutes = played.as_secs() / 60;
    match (minutes / 60, minutes % 60) {
        (0, 0) => crate::i18n::tr("under a minute").into(),
        (0, m) => format!("{m} min"),
        (h, m) if h < 100 => format!("{h} h {m:02} min"),
        (h, _) => format!("{h} h"),
    }
}

/// Turns the game's command into this instance: its own game folder, and a pack's mods. The
/// player's answers that hold everywhere follow it in: data collection and No AI as the
/// launcher has them, and options.txt and the offline name when it has none yet.
pub fn apply(cmd: &mut Command, instance: &Instance) -> Result<(), String> {
    match instance.kind {
        Kind::Default => return Ok(()),
        Kind::Own => {
            fs::create_dir_all(&instance.home)
                .map_err(|e| format!("{}: {e}", instance.home.display()))?;
            cmd.current_dir(&instance.home)
                .env("SUNRISE_HOME", &instance.home);
        }
        Kind::Pack => return Err(registry_pack::play_unavailable()),
    }
    follow_player(&backend::game_home(), &instance.home);
    Ok(())
}

fn follow_player(game_home: &Path, home: &Path) {
    for f in PLAYER_FILES {
        if !home.join(f).exists() && game_home.join(f).is_file() {
            let _ = fs::copy(game_home.join(f), home.join(f));
        }
    }
    let consent = sunrise_launcher_telemetry::consent::Consent::path(game_home);
    if consent.is_file() {
        let to = sunrise_launcher_telemetry::consent::Consent::path(home);
        let _ = fs::create_dir_all(to.parent().unwrap());
        let _ = fs::copy(consent, to);
    }
    let no_ai = crate::no_ai::is_on(game_home);
    if crate::no_ai::is_on(home) != no_ai {
        let _ = crate::no_ai::set(home, no_ai);
    }
}

#[cfg(test)]
#[path = "instances/tests.rs"]
mod tests;
