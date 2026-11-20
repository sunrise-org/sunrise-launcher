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
}

impl Channel {
    /// The channel a game release belongs to. Old versioned pre-releases can be pinned,
    /// but are not followed. The rolling Nightly is a Nightly even when marked for updates.
    pub fn of_release(release: &backend::Release) -> Option<Self> {
        if release.nightly.is_some() || sunrise_launcher_release_verify::is_nightly(&release.tag) {
            Some(Self::Nightly)
        } else if !release.prerelease && backend::is_game_tag(&release.tag) {
            Some(Self::Stable)
        } else {
            None
        }
    }

    /// The downloadable releases on this channel, in catalogue order (newest first).
    pub fn builds(self, releases: &[backend::Release]) -> impl Iterator<Item = &backend::Release> {
        releases
            .iter()
            .filter(move |r| r.asset.is_some() && Self::of_release(r) == Some(self))
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
}

impl Store {
    fn load(data: &Path) -> Store {
        fs::read_to_string(data.join(FILE))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }
    fn save(&self, data: &Path) -> Result<(), String> {
        fs::create_dir_all(data).map_err(|e| format!("{}: {e}", data.display()))?;
        let write = || -> std::io::Result<()> {
            use std::io::Write as _;
            let data = data.canonicalize()?;
            // Own this file before writing to it. A failed save drops just this attempt's
            // file; a successful one replaces the store whole.
            let mut part = tempfile::Builder::new()
                .prefix(".instances-")
                .tempfile_in(&data)?;
            let text = serde_json::to_string_pretty(self).unwrap();
            part.write_all(text.as_bytes())?;
            part.persist(data.join(FILE))?;
            Ok(())
        };
        write().map_err(|e| format!("{FILE}: {e}"))
    }
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
    /// The installed version this instance plays: its pinned one, else `requested` (the
    /// command line's `--version`) or the launcher's (`backend::resolve_version`).
    pub fn resolve_version(&self, requested: Option<&str>) -> Result<String, String> {
        match (requested, &self.version) {
            (None, Some(pinned)) => backend::resolve_version(Some(pinned)).map_err(|_| {
                format!(
                    "{} plays {pinned}, which is not installed: install it, or sunrise-launcher \
                     instance version \"{}\" latest",
                    self.name, self.name
                )
            }),
            _ => backend::resolve_version(requested),
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
    list_in(&backend::game_home(), &backend::data_dir())
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

fn list_in(game_home: &Path, data: &Path) -> Vec<Instance> {
    let mut store = Store::load(data);
    let mut found = vec![(
        DEFAULT.to_string(),
        "Sunrise".to_string(),
        Kind::Default,
        game_home.to_path_buf(),
        game_home.to_path_buf(),
    )];
    let mut own: Vec<_> = fs::read_dir(game_home.join(OWN))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    own.sort();
    // An own instance whose folder is gone (deleted outside the launcher) is forgotten, so a new
    // one of that name starts clean. Only when the folder of instances itself can be read.
    if game_home.join(OWN).is_dir() {
        let before = store.instances.len();
        store
            .instances
            .retain(|id, _| id == DEFAULT || id.contains(':') || own.contains(id));
        if store.instances.len() != before {
            if let Err(e) = store.save(data) {
                eprintln!("Sunrise: instances.json is not saved: {e}");
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
    for p in registry_pack::list_in(&game_home.join("sunrise-packs")) {
        let home = registry_pack::home(&p.work);
        found.push((format!("pack:{}", p.id), p.name, Kind::Pack, p.work, home));
    }
    let mut out: Vec<Instance> = found
        .into_iter()
        .map(|(id, name, kind, work, home)| {
            let meta = store.instances.get(&id).cloned().unwrap_or_default();
            Instance {
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
                work,
                home,
            }
        })
        .collect();
    out[1..].sort_by_key(|i| (i.group.clone(), i.name.to_lowercase()));
    out
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
    let gone = delete_in(
        &backend::game_home(),
        &backend::data_dir(),
        query,
        &crate::desktop_seams::to_trash,
    )?;
    let mut settings = backend::Settings::load();
    let saved = if settings.instance.as_ref() == Some(&gone.id) {
        settings.instance = None;
        settings.save()
    } else {
        Ok(())
    };
    crate::storage::eviction::request_cleanup();
    Ok((gone, saved))
}

fn delete_in(
    game_home: &Path,
    data: &Path,
    query: &str,
    trash: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<Instance, String> {
    let instance = find_in(&list_in(game_home, data), query)?;
    if instance.kind == Kind::Default {
        return Err(format!(
            "{} is the game's own folder: it can't be deleted.",
            instance.name
        ));
    }
    trash(&instance.work)
        .map_err(|e| format!("{} could not go to the Trash: {e}", instance.name))?;
    let mut store = Store::load(data);
    if store.instances.remove(&instance.id).is_some() {
        store.save(data)?;
    }
    Ok(instance)
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
fn retired_in(all: &[Instance], saved: Option<&str>) -> Option<String> {
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

/// The saved selection when it is no longer an instance of this launcher.
pub fn retired_selection() -> Option<String> {
    retired_in(&list(), backend::Settings::load().instance.as_deref())
}

/// The message for a retired selection ([`retired_selection`]).
pub fn retired_selection_message(id: &str) -> String {
    retired_message(id)
}

/// The instance a command without `--instance` acts on: [`selected`], except that a retired
/// selection is an error naming it, so nothing is written to Sunrise's setup in its place.
pub fn selection() -> Result<Instance, String> {
    selection_in(&list(), backend::Settings::load().instance.as_deref())
}

fn selection_in(all: &[Instance], saved: Option<&str>) -> Result<Instance, String> {
    if let Some(id) = retired_in(all, saved) {
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
    create_with(name, from, None, |_| {})
}

/// [`create`], with everything else the New Profile dialog chose, as one step: `picture` (None
/// keeps what a copy brought, `Some(None)` leaves it without one) and `choose`, which sets the
/// rest of its choices. An instance appears only once its folder, picture and choices are all in
/// place; a failure leaves nothing behind, so the same name can be tried again.
pub fn create_with(
    name: &str,
    from: Option<&str>,
    picture: Option<Option<&[u8]>>,
    choose: impl FnOnce(&mut Meta),
) -> Result<Instance, String> {
    let from = from.map(find).transpose()?;
    create_whole_in(
        &backend::game_home(),
        &backend::data_dir(),
        name,
        from.as_ref(),
        picture,
        choose,
    )
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
    let name = name.trim();
    if name.is_empty() {
        return Err("A profile needs a name.".into());
    }
    if let Some(from) = from.filter(|f| f.kind == Kind::Pack) {
        return Err(format!(
            "{} plays its pack's mods: import the pack again for a second copy.",
            from.name
        ));
    }
    let create_error = |e| format!("Could not make profile {name}: {e}");
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
    let stage = tempfile::tempdir_in(&staging).map_err(create_error)?;
    let contents = stage.path().join("home");
    let staged = || -> std::io::Result<()> {
        if let Some(from) = from {
            copy_home(&from.home, &contents)?;
        } else {
            fs::create_dir(&contents)?;
            for f in PLAYER_FILES {
                let source = game_home.join(f);
                if source.is_file() {
                    fs::copy(source, contents.join(f))?;
                }
            }
        }
        let icon = contents.join(PICTURE);
        match picture {
            Some(Some(png)) => fs::write(icon, png)?,
            Some(None) if icon.is_file() => fs::remove_file(icon)?,
            _ => {}
        }
        creation_parent(&root, OWN)?;
        publish_new_folder(&contents, &dir)
    };
    staged().map_err(create_error)?;
    let recorded = crate::run_leases::publish(&dir)
        .map_err(crate::native_mods::cache_error)
        .and_then(|()| {
            let mut store = Store::load(data);
            // A folder removed outside the launcher may still have metadata. A new folder
            // never inherits its launch settings. Do not rewrite the old entry just by listing it.
            store.instances.remove(&slug);
            let meta = store.instances.entry(slug.clone()).or_default();
            meta.name = (name != slug).then(|| name.to_string());
            meta.icon = from.and_then(|f| f.icon.clone());
            meta.group = from.and_then(|f| f.group.clone());
            choose(meta);
            store.save(data)
        });
    if let Err(e) = recorded {
        // A no-replace rename put this attempt's folder here. Don't follow a redirected parent
        // even on the failure path.
        if creation_parent(&root, OWN).is_ok() {
            let _ = fs::remove_dir_all(&dir);
        }
        return Err(e);
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
    if !path.symlink_metadata()?.is_dir() || path.canonicalize()? != path {
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
fn publish_new_folder(from: &Path, to: &Path) -> std::io::Result<()> {
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
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;
        let wide = |path: &Path| {
            path.as_os_str()
                .encode_wide()
                .chain([0])
                .collect::<Vec<_>>()
        };
        let (from, to) = (wide(from), wide(to));
        // SAFETY: both pointers name owned, null-terminated path strings for the call.
        let result = unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileW(from.as_ptr(), to.as_ptr())
        };
        if result != 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (from, to);
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

fn copy_home(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if NOT_COPIED.iter().any(|n| name == *n) {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            copy_dir(&path, &to.join(&name))?;
        } else {
            fs::copy(&path, to.join(&name))?;
        }
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Changes what the player chose about an instance: its name, icon or group. No folder moves.
pub fn edit(query: &str, change: impl FnOnce(&mut Meta)) -> Result<Instance, String> {
    edit_in(&backend::game_home(), &backend::data_dir(), query, change)
}

fn edit_in(
    game_home: &Path,
    data: &Path,
    query: &str,
    change: impl FnOnce(&mut Meta),
) -> Result<Instance, String> {
    let all = list_in(game_home, data);
    let instance = find_in(&all, query)?;
    let mut store = Store::load(data);
    let meta = store.instances.entry(instance.id.clone()).or_default();
    change(meta);
    meta.name = meta.name.take().map(|n| n.trim().to_string());
    meta.group = meta
        .group
        .take()
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty());
    if let Some(name) = &meta.name {
        if name.is_empty() {
            return Err("A profile needs a name.".into());
        }
        // Only a new name is checked: an import can share a name, and changing anything
        // else about it must still work.
        let renamed = !name.eq_ignore_ascii_case(&instance.name);
        if renamed
            && all
                .iter()
                .any(|i| i.id != instance.id && i.name.eq_ignore_ascii_case(name))
        {
            return Err(name_taken(name));
        }
    }
    store.save(data)?;
    find_in(&list_in(game_home, data), &instance.id)
}

/// Adds a finished run of the game to the instance's play time (by id: a renamed instance keeps
/// it).
pub fn record_play(id: &str, played: Duration) -> Result<(), String> {
    record_play_in(&backend::data_dir(), id, played, SystemTime::now())
}

fn record_play_in(
    data: &Path,
    id: &str,
    played: Duration,
    ended: SystemTime,
) -> Result<(), String> {
    let mut store = Store::load(data);
    let meta = store.instances.entry(id.to_string()).or_default();
    meta.played_secs = meta.played_secs.saturating_add(played.as_secs());
    meta.last_played = ended.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs());
    store.save(data)
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
        Kind::Pack => registry_pack::apply(cmd, &instance.work)?,
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
