//! An instance's single-player worlds (`saves/` in its game folder): what each is, and backing
//! up, copying and deleting them. Backups are made as vanilla's Make Backup makes them
//! (`LevelStorageAccess.makeWorldBackup`): `backups/<yyyy-MM-dd_HH-mm-ss>_<folder>.zip` in the
//! game folder, every file under `<folder>/` but the world's `session.lock`.
use crate::profile_txn::Identity;
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt};
use cap_std::{ambient_authority, fs::Dir};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use sunrise_launcher_nbt::{self as nbt, Limits, Tag};
use sunrise_launcher_smod::{manifest::revision, runtime_leases::linked};

mod effects;
pub(crate) use effects::update_drops;
pub use effects::{unproved_mod_effects, SuspectMods};

pub(crate) mod switch;

/// The lock a running game holds on a world; vanilla's backups leave it out.
pub(crate) const LOCK: &str = "session.lock";
/// The latest `LastPlayed` kept: the end of year 9999, in milliseconds. A later time does not fit a
/// Windows `SystemTime`, and adding it panics.
const MAX_LAST_PLAYED_MILLIS: u64 = 253_402_300_799_000;

pub(crate) struct WorldLock(fs::File);

impl Drop for WorldLock {
    fn drop(&mut self) {
        // A concurrent process spawn can inherit the file before exec closes it.
        // Release the lock explicitly rather than waiting for every handle to close.
        let _ = self.0.unlock();
    }
}

/// Hold the same exclusive lock as the game until a filesystem operation finishes.
fn lock_world(world: &World) -> Result<WorldLock, String> {
    lock_saved_dir(&world.path)
}

pub(crate) fn lock_saved_dir(world: &Path) -> Result<WorldLock, String> {
    let error = |e: &dyn std::fmt::Display| {
        crate::i18n::trf(
            "Could not lock world {world}: {error}",
            &[
                (
                    "world",
                    &world.file_name().unwrap_or_default().to_string_lossy(),
                ),
                ("error", &e.to_string()),
            ],
        )
    };
    let parent = world
        .parent()
        .ok_or_else(|| error(&std::io::ErrorKind::InvalidInput))?;
    for dir in [parent, world] {
        let folder = fs::symlink_metadata(dir).map_err(|e| error(&e))?;
        if !folder.is_dir() || crate::native_cache::linked(&folder) {
            return Err(error(&std::io::ErrorKind::InvalidInput));
        }
    }
    let path = world.join(LOCK);
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_file() && !crate::native_cache::linked(&meta) => (),
        Ok(_) => return Err(error(&std::io::ErrorKind::InvalidInput)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(error(&e)),
    }
    let lock = fs::File::options()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| error(&e))?;
    lock.try_lock().map_err(|e| error(&e))?;
    Ok(WorldLock(lock))
}

/// Each world in a profile snapshot must be the directory whose session lock we hold.
pub(crate) struct SavedWorlds(BTreeMap<PathBuf, (Identity, WorldLock)>);

impl SavedWorlds {
    pub(crate) fn claim(saves: &Path) -> Result<Self, String> {
        let mut claims = BTreeMap::new();
        let entries = match fs::symlink_metadata(saves) {
            Ok(meta) if meta.is_dir() && !crate::native_cache::linked(&meta) => fs::read_dir(saves),
            Ok(_) => Err(std::io::ErrorKind::InvalidInput.into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self(claims)),
            Err(e) => Err(e),
        }
        .map_err(|e| e.to_string())?;
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            if fs::symlink_metadata(&path)
                .map_err(|e| e.to_string())?
                .is_dir()
            {
                let identity = Identity::of(&path)?;
                let lock = lock_saved_dir(&path)?;
                claims.insert(path, (identity, lock));
            }
        }
        Ok(Self(claims))
    }

    pub(crate) fn verify(&self, world: &Path) -> std::io::Result<()> {
        if self
            .0
            .get(world)
            .is_some_and(|(identity, _)| Identity::of(world).ok() == Some(*identity))
        {
            return Ok(());
        }
        Err(std::io::Error::other(crate::i18n::trf(
            "World {world} changed while copying. Close the game and try again.",
            &[(
                "world",
                &world.file_name().unwrap_or_default().to_string_lossy(),
            )],
        )))
    }

    pub(crate) fn verify_all(&self) -> std::io::Result<()> {
        self.0.keys().try_for_each(|path| self.verify(path))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct World {
    /// Its folder's name under `saves/`, the id commands take.
    pub folder: String,
    pub path: PathBuf,
    /// `LevelName`, else the folder's name.
    pub name: String,
    /// Survival, Creative, Adventure, Spectator or Hardcore.
    pub mode: &'static str,
    /// `LastPlayed`, else when level.dat changed.
    pub last_played: SystemTime,
    /// The version that last saved it (`Version.Name`).
    pub version: Option<String>,
    /// Bytes on disk, every file in its folder.
    pub size: u64,
    /// `icon.png`, the picture the game took of it.
    pub icon: Option<PathBuf>,
    /// The ids of the mods it was last saved with: the code mods of `fml.LoadingModList`, then the
    /// content mods of Sunrise's `sunrise.ContentModList`, as the game writes them.
    pub mods: Vec<String>,
}

/// The worlds of `worlds` saved with any of the mods `ids`, each with those of its mods.
pub fn saved_with<'a>(worlds: &'a [World], ids: &[String]) -> Vec<(&'a World, Vec<String>)> {
    worlds
        .iter()
        .filter_map(|world| {
            let used: Vec<String> = ids
                .iter()
                .filter(|id| world.mods.contains(id))
                .cloned()
                .collect();
            (!used.is_empty()).then_some((world, used))
        })
        .collect()
}

fn list_raw(home: &Path) -> Vec<World> {
    let mut worlds: Vec<Scanned> = fs::read_dir(home.join("saves"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| scanned_world(&e.path()).ok().flatten())
        .collect();
    // Worlds played at the same moment go by folder name, not by the order the disk lists them.
    worlds.sort_by(|a, b| {
        b.played
            .cmp(&a.played)
            .then_with(|| a.world.folder.cmp(&b.world.folder))
    });
    worlds.into_iter().map(|s| s.world).collect()
}

/// The worlds in a game folder's `saves/`, the last played first.
pub fn list(home: &Path) -> Vec<World> {
    let worlds = list_raw(home);
    // A world made since the launcher started is hidden from its log as well.
    for world in &worlds {
        crate::launcher_log::hide(world.folder.as_str());
        crate::launcher_log::hide(world.name.as_str());
    }
    worlds
}

/// Whether `folder` has a level.dat file the game's reader can read (no recovery from level.dat_old).
pub(crate) fn has_readable_level_dat(folder: &Path) -> bool {
    let dat = folder.join("level.dat");
    dat.is_file() && level_dat(&dat).is_some()
}

/// level.dat: gzip, as vanilla writes it, or plain. Read with the limits the game reads a world's
/// `.dat` files with ([`Limits::saved`]: its depth, no cap on the tag count, and a size bound), so
/// a level.dat the game loads is never taken for an unreadable one.
pub(crate) fn level_dat(path: &Path) -> Option<Tag> {
    decode_level_dat(&fs::read(path).ok()?)
}

fn decode_level_dat(bytes: &[u8]) -> Option<Tag> {
    let limits = Limits::saved();
    let mut raw = Vec::new();
    let raw = match flate2::read::GzDecoder::new(bytes)
        .take(limits.max_bytes as u64 + 1)
        .read_to_end(&mut raw)
    {
        Ok(_) if raw.len() <= limits.max_bytes => &raw,
        Ok(_) => return None,
        Err(_) => bytes,
    };
    Some(nbt::read_named(raw, limits).ok()?.0.tag)
}

fn read(path: &Path) -> Option<World> {
    scanned_world(path).ok().flatten().map(|s| s.world)
}

/// A folder with either level file is a world. Display metadata follows the game's recovery order
/// (`level.dat`, then `level.dat_old`), while mod lists are unioned across both readable files so
/// a mod recorded only in `level.dat_old` is never missed. If neither is readable, the world's
/// mod list is unknown.
fn scanned_world(path: &Path) -> std::io::Result<Option<Scanned>> {
    let mut changed = None;
    let mut roots: Vec<Tag> = Vec::new();
    for file in ["level.dat", "level.dat_old"] {
        let dat = path.join(file);
        let meta = match fs::symlink_metadata(&dat) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        changed.get_or_insert_with(|| meta.modified().unwrap_or(UNIX_EPOCH));
        if dat.is_file() {
            if let Some(parsed) =
                level_dat(&dat).filter(|r| matches!(r.get("Data"), Some(Tag::Compound(_))))
            {
                roots.push(parsed);
            }
        }
    }
    let Some(changed) = changed else {
        return Ok(None);
    };
    let root = roots.first();
    let folder = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let known = root.is_some();
    let data = root.and_then(|r| r.get("Data"));
    let field = |key: &str| data.and_then(|d| d.get(key));
    let number = |tag: Option<&Tag>| match tag {
        Some(Tag::Byte(v)) => i64::from(*v),
        Some(Tag::Short(v)) => i64::from(*v),
        Some(Tag::Int(v)) => i64::from(*v),
        Some(Tag::Long(v)) => *v,
        // As the game's numeric tags give a long: a float's `longValue` casts, so it truncates
        // toward zero (-0.5 is 0), while a double's floors (-0.5 is -1). A NaN is 0.
        Some(Tag::Float(bits)) => f64::from(f32::from_bits(*bits)).trunc() as i64,
        Some(Tag::Double(bits)) => f64::from_bits(*bits).floor() as i64,
        _ => 0,
    };
    let text = |tag: Option<&Tag>| match tag {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    };
    // 26.3 keeps hardcore with the difficulty; older saves at the top of Data.
    let hardcore = number(field("difficulty_settings").and_then(|d| d.get("hardcore"))) != 0
        || number(field("hardcore")) != 0;
    let mode = match (hardcore, number(field("GameType"))) {
        (true, _) => crate::i18n::tr_key("Hardcore"),
        (_, 1) => crate::i18n::tr_key("Creative"),
        (_, 2) => crate::i18n::tr_key("Adventure"),
        (_, 3) => crate::i18n::tr_key("Spectator"),
        _ => crate::i18n::tr_key("Survival"),
    };
    let (played, last_played) = match number(field("LastPlayed")) {
        millis if millis > 0 => (
            Duration::from_millis(millis as u64),
            UNIX_EPOCH + Duration::from_millis((millis as u64).min(MAX_LAST_PLAYED_MILLIS)),
        ),
        _ => (
            changed.duration_since(UNIX_EPOCH).unwrap_or_default(),
            changed,
        ),
    };
    let icon = path.join("icon.png");
    // `[{ModId, ModVersion}]` lists, as `save::level_mods` writes them; anything else reads as none.
    let listed = |list: Option<&Tag>| -> Vec<String> {
        let Some(Tag::List(entries)) = list else {
            return Vec::new();
        };
        entries
            .iter()
            .filter_map(|entry| text(entry.get("ModId")))
            .filter(|id| !id.is_empty())
            .collect()
    };
    let mut mods = Vec::new();
    for level in &roots {
        for id in listed(level.get("fml").and_then(|fml| fml.get("LoadingModList")))
            .into_iter()
            .chain(listed(
                level
                    .get("sunrise")
                    .and_then(|sunrise| sunrise.get("ContentModList")),
            ))
        {
            if !mods.contains(&id) {
                mods.push(id);
            }
        }
    }
    let world = World {
        name: text(field("LevelName"))
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| folder.clone()),
        folder,
        path: path.to_path_buf(),
        mode,
        last_played,
        version: text(field("Version").and_then(|v| v.get("Name"))),
        size: size(path),
        icon: icon.is_file().then_some(icon),
        mods,
    };
    Ok(Some(Scanned {
        world,
        known,
        played,
    }))
}

/// A world in `saves/` and whether the list of its mods could be read. A world whose level.dat
/// and level.dat_old both fail to read is *unknown*: it counts as holding every mod, never none.
#[derive(Clone, Debug)]
struct Scanned {
    pub world: World,
    pub known: bool,
    /// `LastPlayed` as saved, the order of [`list`]: `world.last_played` is clamped, which would
    /// lose the order of two worlds played past the clamp.
    pub played: Duration,
}

/// Every world of `home`'s `saves/`, failing closed. A missing `saves/` is no worlds; any other
/// failure to list the folder, an entry or a level file's metadata is an error, never "no
/// worlds". Unlike [`list`], which skips what it can't read for the player's screen, this is for
/// the changes that must not lose a world's mods.
fn scan(home: &Path) -> Result<Vec<Scanned>, String> {
    let saves = home.join("saves");
    let entries = match fs::read_dir(&saves) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", saves.display())),
    };
    let mut out = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|e| format!("{}: {e}", saves.display()))?
            .path();
        if !path
            .metadata()
            .map_err(|e| format!("{}: {e}", path.display()))?
            .is_dir()
        {
            continue;
        }
        if let Some(world) = scanned_world(&path).map_err(|e| format!("{}: {e}", path.display()))? {
            out.push(world);
        }
    }
    Ok(out)
}

/// Saved worlds a change to these mods could affect. Unknown worlds count as saved with every one of them.
pub fn at_risk(home: &Path, ids: &[String]) -> Result<Vec<(World, Vec<String>, bool)>, String> {
    Ok(scan(home)?
        .into_iter()
        .filter_map(|s| {
            let used: Vec<_> = ids
                .iter()
                .filter(|id| !s.known || s.world.mods.contains(id))
                .cloned()
                .collect();
            (!used.is_empty()).then_some((s.world, used, s.known))
        })
        .collect())
}

/// One admitted save's risk for a proposed change. Equality binds the directory and the exact
/// bytes (or absence) of both level files, not just the fields shown in the warning.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldRisk {
    pub path: PathBuf,
    pub name: String,
    /// All suspect IDs, in ID order. A last-loaded list cannot rule out historical use.
    pub mods: Vec<String>,
    /// Suspects positively identified in either level file, in ID order.
    pub confirmed_mods: Vec<String>,
    /// Every ID in `mods` has positive evidence. False means some of the risk is uncertain.
    pub known: bool,
    /// `level.dat` contains at least one nonempty, valid mod list and no malformed mod evidence.
    /// This describes the last-loaded set only, never whether historic chunks contain a mod.
    pub primary_complete: bool,
    /// A present level file could not be parsed or has malformed mod evidence. Omitted or empty
    /// lists are not malformed. Positive evidence in other lists or files is still reported.
    pub malformed: bool,
    /// Build-order evidence comes only from `level.dat`, never the older recovery file.
    pub data_version: Option<i32>,
    pub saved_commit: Option<String>,
    directory: DirectoryIdentity,
    levels: [Option<[u8; 32]>; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct DirectoryIdentity {
    device: u64,
    inode: u64,
    created: Option<cap_std::time::SystemTime>,
}

impl DirectoryIdentity {
    fn of(dir: &Dir) -> io::Result<Self> {
        let meta = dir.dir_metadata()?;
        Ok(Self {
            device: meta.dev(),
            inode: meta.ino(),
            created: meta.created().ok(),
        })
    }
}

/// Read a path-ordered world roster under the caller's profile admission. `sources` maps suspect
/// mod IDs to namespaces claimed by their definitions. Both level files can confirm use, but
/// neither can rule it out: chunks can retain blocks and items from mods no longer loaded. Every
/// identified world therefore carries all suspects, with positive matches tracked separately.
/// A directory with neither level file isn't a discovered world.
///
/// Fails on traversal or read I/O errors, redirected directories and aliased evidence files. A
/// file whose bytes could be read and hashed but not parsed sets [`WorldRisk::malformed`] without
/// suppressing recoverable positive evidence. This reader does not create or acquire `session.lock`.
/// Callers hold profile admission (including roster writers), obtain
/// [`lock_reviewed`] before publishing, and compare a fresh snapshot while both are held.
/// Paths above the admitted home, same-volume mount aliases without a second visible name and
/// arbitrary external writes are outside this check.
pub fn review_risk(
    home: &Path,
    sources: &BTreeMap<String, BTreeSet<String>>,
) -> Result<Vec<WorldRisk>, String> {
    let meta = fs::symlink_metadata(home).map_err(|e| world_error(home, e))?;
    if linked(&meta) || !meta.is_dir() {
        return Err(world_error(home, io::ErrorKind::InvalidData));
    }
    let root =
        Dir::open_ambient_dir(home, ambient_authority()).map_err(|e| world_error(home, e))?;
    let root_id = DirectoryIdentity::of(&root).map_err(|e| world_error(home, e))?;
    let saves_path = home.join("saves");
    match fs::symlink_metadata(&saves_path) {
        Ok(meta) if meta.is_dir() && !linked(&meta) => (),
        Ok(_) => return Err(world_error(&saves_path, io::ErrorKind::InvalidData)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(world_error(&saves_path, e)),
    }
    let saves = root
        .open_dir_nofollow("saves")
        .map_err(|e| world_error(&saves_path, e))?;
    let saves_id = DirectoryIdentity::of(&saves).map_err(|e| world_error(&saves_path, e))?;
    if saves_id.device != root_id.device || saves_id == root_id {
        return Err(world_error(&saves_path, io::ErrorKind::InvalidData));
    }
    let mut seen = BTreeSet::from([root_id, saves_id]);
    let mut worlds = Vec::new();
    for entry in saves.entries().map_err(|e| world_error(&saves_path, e))? {
        let folder = entry.map_err(|e| world_error(&saves_path, e))?.file_name();
        let path = saves_path.join(&folder);
        let meta = fs::symlink_metadata(&path).map_err(|e| world_error(&path, e))?;
        if linked(&meta) {
            return Err(world_error(&path, io::ErrorKind::InvalidData));
        }
        if !meta.is_dir() {
            continue;
        }
        let dir = saves
            .open_dir_nofollow(&folder)
            .map_err(|e| world_error(&path, e))?;
        let directory = DirectoryIdentity::of(&dir).map_err(|e| world_error(&path, e))?;
        if directory.device != saves_id.device || !seen.insert(directory) {
            return Err(world_error(&path, io::ErrorKind::InvalidData));
        }
        let read = |name: &str| {
            let file = path.join(name);
            read_level_evidence(&dir, name, &file, directory.device)
                .map_err(|e| world_error(&file, e))
        };
        let primary = read("level.dat")?;
        let fallback = read("level.dat_old")?;
        if primary.is_none() && fallback.is_none() {
            continue;
        }
        let primary_records = primary.as_ref().and_then(|level| level.records.as_ref());
        let fallback_records = fallback.as_ref().and_then(|level| level.records.as_ref());
        let primary_complete = primary_records.is_some_and(|r| r.valid && !r.ids.is_empty());
        let malformed = [primary.as_ref(), fallback.as_ref()]
            .into_iter()
            .flatten()
            .any(|file| !file.records.as_ref().is_some_and(|r| r.valid));
        let confirmed_mods: Vec<String> = sources
            .iter()
            .filter(|(id, claims)| {
                [primary_records, fallback_records]
                    .into_iter()
                    .flatten()
                    .any(|r| {
                        r.ids.contains(id.as_str())
                            || claims
                                .iter()
                                .any(|ns| ns != "minecraft" && r.namespaces.contains(ns))
                    })
            })
            .map(|(id, _)| id.clone())
            .collect();
        worlds.push(WorldRisk {
            path,
            name: primary_records
                .and_then(|r| r.name.clone())
                .or_else(|| fallback_records.and_then(|r| r.name.clone()))
                .unwrap_or_else(|| folder.to_string_lossy().into_owned()),
            known: confirmed_mods.len() == sources.len(),
            mods: sources.keys().cloned().collect(),
            confirmed_mods,
            primary_complete,
            malformed,
            data_version: primary_records.and_then(|r| r.data_version),
            saved_commit: primary_records.and_then(|r| r.saved_commit.clone()),
            directory,
            levels: [
                primary.map(|file| file.digest),
                fallback.map(|file| file.digest),
            ],
        });
    }
    worlds.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(worlds)
}

/// Held game-compatible locks for an admitted world roster. Dropping this guard releases them.
pub struct ReviewedWorldLocks {
    _locks: Vec<WorldLock>,
}

/// Lock the exact reviewed directories in path order without following redirected directories or
/// aliased lock files. If any lock can't be acquired, release the rest and return an error.
/// The caller keeps profile admission, rechecks the entire roster with [`review_risk`] after
/// acquiring these locks and holds this guard through publishing. These locks exclude Sunrise's
/// world opener, not arbitrary external file edits or new children in `saves`.
pub fn lock_reviewed(worlds: &[WorldRisk]) -> Result<ReviewedWorldLocks, String> {
    let mut ordered: Vec<_> = worlds.iter().collect();
    ordered.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    let mut locks = Vec::with_capacity(ordered.len());
    for world in ordered {
        let path = &world.path;
        let parent = path
            .parent()
            .ok_or_else(|| world_error(path, io::ErrorKind::InvalidInput))?;
        let folder = path
            .file_name()
            .ok_or_else(|| world_error(path, io::ErrorKind::InvalidInput))?;
        let home = parent
            .parent()
            .ok_or_else(|| world_error(parent, io::ErrorKind::InvalidInput))?;
        let saves_name = parent
            .file_name()
            .ok_or_else(|| world_error(parent, io::ErrorKind::InvalidInput))?;
        for directory in [home, parent, path] {
            let meta = fs::symlink_metadata(directory).map_err(|e| world_error(directory, e))?;
            if linked(&meta) || !meta.is_dir() {
                return Err(world_error(directory, io::ErrorKind::InvalidData));
            }
        }
        let dir = Dir::open_ambient_dir(home, ambient_authority())
            .and_then(|home| home.open_dir_nofollow(saves_name))
            .and_then(|saves| saves.open_dir_nofollow(folder))
            .map_err(|e| world_error(path, e))?;
        if DirectoryIdentity::of(&dir).map_err(|e| world_error(path, e))? != world.directory {
            return Err(world_error(path, "The reviewed world directory changed"));
        }
        let lock = path.join(LOCK);
        let mut options = cap_std::fs::OpenOptions::new();
        options.write(true).follow(FollowSymlinks::No);
        match fs::symlink_metadata(&lock) {
            Ok(meta) if meta.is_file() && !linked(&meta) => (),
            Ok(_) => return Err(world_error(&lock, io::ErrorKind::InvalidData)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                options.create_new(true);
            }
            Err(e) => return Err(world_error(&lock, e)),
        }
        let file = dir
            .open_with(LOCK, &options)
            .map_err(|e| world_error(&lock, e))?;
        let meta = file.metadata().map_err(|e| world_error(&lock, e))?;
        if !meta.is_file() || meta.nlink() != 1 || meta.dev() != world.directory.device {
            return Err(world_error(&lock, io::ErrorKind::InvalidData));
        }
        let file = file.into_std();
        file.try_lock().map_err(|e| world_error(&lock, e))?;
        locks.push(WorldLock(file));
    }
    Ok(ReviewedWorldLocks { _locks: locks })
}

fn world_error(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

struct LevelEvidence {
    digest: [u8; 32],
    records: Option<LevelRecords>,
}

/// The hash and parsed evidence must come from the same read, so a changed file can't contribute
/// metadata to an earlier snapshot. A missing file is distinct from a present, malformed one.
fn read_level_evidence(
    dir: &Dir,
    name: &str,
    path: &Path,
    device: u64,
) -> io::Result<Option<LevelEvidence>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && !linked(&meta) => (),
        Ok(_) => return Err(io::ErrorKind::InvalidData.into()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    }
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = dir.open_with(name, &options)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.nlink() != 1 || meta.dev() != device {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let limit = Limits::saved().max_bytes;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "World evidence exceeds the save size limit",
        ));
    }
    Ok(Some(LevelEvidence {
        digest: Sha256::digest(&bytes).into(),
        records: decode_level_dat(&bytes).and_then(|root| LevelRecords::parse(&root)),
    }))
}

struct LevelRecords {
    name: Option<String>,
    data_version: Option<i32>,
    saved_commit: Option<String>,
    ids: BTreeSet<String>,
    namespaces: BTreeSet<String>,
    valid: bool,
}

impl LevelRecords {
    fn parse(root: &Tag) -> Option<Self> {
        let data @ Tag::Compound(_) = root.get("Data")? else {
            return None;
        };
        let sunrise = root.get("sunrise");
        let fml = root.get("fml");
        let shape_ok = [sunrise, fml]
            .into_iter()
            .flatten()
            .all(|tag| matches!(tag, Tag::Compound(_)));
        let mod_id = |entry: &Tag| {
            // Versions are opaque saved text; the game writes "" when not known.
            saved_string(entry.get("ModVersion")).filter(|v| !v.chars().any(char::is_control))?;
            saved_string(entry.get("ModId"))
                .filter(|id| (1..=64).contains(&id.len()) && id.bytes().all(mod_id_byte))
        };
        let content = validated_list(sunrise.and_then(|tag| tag.get("ContentModList")), mod_id);
        let legacy = validated_list(fml.and_then(|tag| tag.get("LoadingModList")), mod_id);
        let namespaces = validated_list(
            sunrise.and_then(|tag| tag.get("BlockNamespaces")),
            |entry| {
                saved_string(Some(entry)).filter(|ns| {
                    !ns.is_empty()
                        && ns
                            .bytes()
                            .all(|b| mod_id_byte(b) || matches!(b, b'.' | b'-'))
                })
            },
        );
        Some(Self {
            name: saved_string(data.get("LevelName")).filter(|name| !name.trim().is_empty()),
            data_version: match data.get("DataVersion") {
                Some(Tag::Int(version)) => Some(*version),
                _ => None,
            },
            saved_commit: saved_string(sunrise.and_then(|tag| tag.get("LastSavedCommit")))
                .filter(|commit| revision(commit)),
            valid: shape_ok && content.is_some() && legacy.is_some() && namespaces.is_some(),
            ids: content.into_iter().chain(legacy).flatten().collect(),
            namespaces: namespaces.unwrap_or_default(),
        })
    }
}

fn mod_id_byte(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'
}

fn saved_string(tag: Option<&Tag>) -> Option<String> {
    match tag {
        Some(Tag::String(s)) => s.to_string().ok(),
        _ => None,
    }
}

/// The game omits empty lists. Reject a malformed list as a unit; other lists and the other
/// level file can still supply positive evidence.
fn validated_list(
    tag: Option<&Tag>,
    parse: impl Fn(&Tag) -> Option<String>,
) -> Option<BTreeSet<String>> {
    let Some(tag) = tag else {
        return Some(BTreeSet::new());
    };
    let Tag::List(entries) = tag else {
        return None;
    };
    let mut found = BTreeSet::new();
    for entry in entries {
        if !found.insert(parse(entry)?) {
            return None;
        }
    }
    Some(found)
}

fn size(dir: &Path) -> u64 {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => size(&e.path()),
            Ok(_) => e.metadata().map_or(0, |m| m.len()),
            Err(_) => 0,
        })
        .sum()
}

fn find_in(all: &[World], home: &Path, query: &str) -> Result<World, String> {
    all.iter()
        .find(|w| w.folder == query)
        .or_else(|| all.iter().find(|w| w.name.eq_ignore_ascii_case(query)))
        .cloned()
        .ok_or_else(|| format!("No world {query} in {}", home.join("saves").display()))
}

fn find_raw(home: &Path, query: &str) -> Result<World, String> {
    find_in(&list_raw(home), home, query)
}

/// The world in `home` a command names: its folder, or its name (any case).
pub fn find(home: &Path, query: &str) -> Result<World, String> {
    find_in(&list(home), home, query)
}

/// `base`, else `base (1)`, `base (2)`... from the `first` count: the first name free in `dir`
/// (vanilla's `FileUtil.findAvailableName`, after making the name safe for every file system).
fn free_name(dir: &Path, base: &str, suffix: &str, first: u32) -> String {
    let safe: String = base
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let safe = safe.trim_matches(|c: char| c == '.' || c.is_whitespace());
    let safe = if safe.is_empty() { "World" } else { safe };
    (first..)
        .map(|n| match n {
            0 => format!("{safe}{suffix}"),
            n => format!("{safe} ({n}){suffix}"),
        })
        .find(|name| !dir.join(name).exists())
        .expect("a folder can't hold a world under every numbered name")
}

/// Zips the world into the game folder's `backups/`, as vanilla's Make Backup does. Returns the
/// zip's path.
pub fn backup(home: &Path, world: &World) -> Result<PathBuf, String> {
    let _lock = lock_world(world)?;
    backup_at(home, world, chrono::Local::now())
}

fn backup_at(
    home: &Path,
    world: &World,
    now: chrono::DateTime<chrono::Local>,
) -> Result<PathBuf, String> {
    let dir = home.join("backups");
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let prefix = format!("{}_{}", now.format("%Y-%m-%d_%H-%M-%S"), world.folder);
    let zip_path = dir.join(free_name(&dir, &prefix, ".zip", 0));
    let made = (|| -> std::io::Result<()> {
        let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(fs::File::create(&zip_path)?));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .large_file(true);
        let mut stack = vec![world.path.clone()];
        while let Some(dir) = stack.pop() {
            let mut entries: Vec<_> = fs::read_dir(&dir)?.collect::<Result<_, _>>()?;
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                let path = e.path();
                if e.file_type()?.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.file_name() == Some(LOCK.as_ref()) {
                    continue;
                }
                let relative = path.strip_prefix(&world.path).unwrap_or(&path);
                let mut entry = world.folder.clone();
                for part in relative.components() {
                    entry.push('/');
                    entry.push_str(&part.as_os_str().to_string_lossy());
                }
                zip.start_file(entry, options)?;
                std::io::copy(&mut fs::File::open(&path)?, &mut zip)?;
            }
        }
        zip.finish()?.flush()
    })();
    made.map(|()| zip_path.clone()).map_err(|e| {
        let _ = fs::remove_file(&zip_path);
        format!("Could not back up {}: {e}", world.name)
    })
}

/// Copies the world into a new folder of the same `saves/`, named "<name> (1)", "<name> (2)"...
/// and gives the copy that name in its level.dat so the game's list tells them apart.
pub fn copy(instance: &crate::instances::Instance, query: &str) -> Result<World, String> {
    let profile = crate::managed_play::Profile::open_selected(instance)?;
    let _ = list(profile.home());
    let copied = copy_admitted(profile, query, |_| {})?;
    crate::launcher_log::hide(copied.folder.as_str());
    crate::launcher_log::hide(copied.name.as_str());
    Ok(copied)
}

fn copy_admitted(
    profile: crate::managed_play::Profile,
    query: &str,
    before_publish: impl FnOnce(&Path),
) -> Result<World, String> {
    use crate::profile_txn::durable;
    let home = profile.home();
    let world = find_raw(home, query)?;
    let saves = home.join("saves");
    let error = |e: std::io::Error| {
        crate::i18n::trf(
            "Could not copy {world}: {error}",
            &[("world", &world.name), ("error", &e.to_string())],
        )
    };
    for path in [&saves, &world.path] {
        let meta = fs::symlink_metadata(path).map_err(error)?;
        if !meta.is_dir() || crate::native_cache::linked(&meta) {
            return Err(error(std::io::ErrorKind::InvalidData.into()));
        }
    }
    let _lock = lock_world(&world)?;
    let folder = free_name(&saves, &world.name, "", 1);
    let to = saves.join(&folder);
    let stage = tempfile::Builder::new()
        .prefix(".world-copy-")
        .tempdir_in(&saves)
        .map_err(error)?;
    let candidate = stage.path().join("world");
    copy_dir(&world.path, &candidate)
        .and_then(|()| rename_level(&candidate, &folder))
        .and_then(|()| durable::flush_tree(&candidate))
        .map_err(error)?;
    before_publish(&to);
    durable::move_new_directory(&candidate, &to).map_err(error)?;
    read(&to).ok_or_else(|| {
        crate::i18n::trf(
            "{folder} has no level.dat",
            &[("folder", &to.to_string_lossy())],
        )
    })
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == LOCK {
            continue;
        }
        let path = entry.path();
        let meta = fs::symlink_metadata(&path)?;
        if crate::native_cache::linked(&meta) {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        let target = to.join(name);
        if meta.is_dir() {
            copy_dir(&path, &target)?;
        } else if meta.is_file() {
            fs::copy(&path, target)?;
        } else {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
    }
    Ok(())
}

/// Writes `root` as level.dat the way the game does (`LevelStorageSource.saveDataTag`): gzip into
/// a temp file in the world folder, the old level.dat renamed to `level.dat_old`, then the temp file
/// renamed to level.dat. The caller holds the world's lock ([`lock_saved_dir`]).
pub(crate) fn save_level_dat(world: &Path, root: Tag) -> io::Result<()> {
    let bytes = nbt::write_named(
        &nbt::NamedTag {
            name: "".into(),
            tag: root,
        },
        Limits::saved(),
    )
    .map_err(io::Error::other)?;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&bytes)?;
    let mut temp = tempfile::Builder::new()
        .prefix("level")
        .suffix(".dat")
        .tempfile_in(world)?;
    temp.write_all(&gz.finish()?)?;
    let dat = world.join("level.dat");
    let old = world.join("level.dat_old");
    let had_dat = dat.exists();
    if had_dat {
        match fs::remove_file(&old) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => (),
        }
        fs::rename(&dat, &old)?;
    }
    if let Err(e) = temp.persist(&dat) {
        if had_dat {
            let _ = fs::rename(&old, &dat);
        }
        return Err(e.error);
    }
    Ok(())
}

/// Sets `Data.LevelName`; a level.dat that can't be read is left as it is.
fn rename_level(world: &Path, name: &str) -> std::io::Result<()> {
    let dat = world.join("level.dat");
    let Some(mut root) = level_dat(&dat) else {
        return Ok(());
    };
    let Some(Tag::Compound(data)) = root.get_mut("Data") else {
        return Ok(());
    };
    data.insert("LevelName".into(), Tag::String(name.into()));
    let bytes = nbt::write_named(
        &nbt::NamedTag {
            name: "".into(),
            tag: root,
        },
        Limits::default(),
    )
    .map_err(std::io::Error::other)?;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&bytes)?;
    fs::write(dat, gz.finish()?)
}

/// Moves the world to the system's trash, where the player can still get it back.
pub fn delete(instance: &crate::instances::Instance, query: &str) -> Result<World, String> {
    let profile = crate::managed_play::Profile::open_selected(instance)?;
    let world = find(profile.home(), query)?;
    delete_world(&world)?;
    Ok(world)
}

fn delete_world(world: &World) -> Result<(), String> {
    let lock = lock_world(world)?;
    // Match server deletion: Windows cannot move the folder with this handle open.
    // If the game opens it after the probe, its handle prevents the move instead.
    #[cfg(windows)]
    drop(lock);
    #[cfg(not(windows))]
    let _lock = lock;
    crate::desktop_seams::to_trash(&world.path)
        .map_err(|e| format!("Could not move {} to the trash: {e}", world.name))
}

#[cfg(test)]
mod tests;
