//! An instance's single-player worlds (`saves/` in its game folder): what each is, and backing
//! up, copying and deleting them. Backups are made as vanilla's Make Backup makes them
//! (`LevelStorageAccess.makeWorldBackup`): `backups/<yyyy-MM-dd_HH-mm-ss>_<folder>.zip` in the
//! game folder, every file under `<folder>/` but the world's `session.lock`.
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use sunrise_launcher_nbt::{self as nbt, Limits, Tag};

pub(crate) mod switch;
pub use switch::Question as SwitchQuestion;

/// The lock a running game holds on a world; vanilla's backups leave it out.
const LOCK: &str = "session.lock";
/// The latest `LastPlayed` kept: the end of year 9999, in milliseconds. A later time does not fit a
/// Windows `SystemTime`, and adding it panics.
const MAX_LAST_PLAYED_MILLIS: u64 = 253_402_300_799_000;

struct WorldLock(fs::File);

impl Drop for WorldLock {
    fn drop(&mut self) {
        // A concurrent process spawn can inherit the file before exec closes it.
        // Release the lock explicitly rather than waiting for every handle to close.
        let _ = self.0.unlock();
    }
}

/// Hold the same exclusive lock as the game until a filesystem operation finishes.
fn lock_world(world: &World) -> Result<WorldLock, String> {
    let lock = fs::File::options()
        .write(true)
        .create(true)
        .truncate(false)
        .open(world.path.join(LOCK))
        .map_err(|e| e.to_string())?;
    lock.try_lock().map_err(|e| e.to_string())?;
    Ok(WorldLock(lock))
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

/// The worlds in a game folder's `saves/`, the last played first.
pub fn list(home: &Path) -> Vec<World> {
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

/// Whether `folder` has a level.dat file the game's reader can read (no recovery from level.dat_old).
pub(crate) fn has_readable_level_dat(folder: &Path) -> bool {
    let dat = folder.join("level.dat");
    dat.is_file() && level_dat(&dat).is_some()
}

/// level.dat: gzip, as vanilla writes it, or plain. Read with the limits the game reads a world's
/// `.dat` files with ([`Limits::saved`]: its depth, no cap on the tag count, and a size bound), so
/// a level.dat the game loads is never taken for an unreadable one.
fn level_dat(path: &Path) -> Option<Tag> {
    let limits = Limits::saved();
    let bytes = fs::read(path).ok()?;
    let mut raw = Vec::new();
    let raw = match flate2::read::GzDecoder::new(&bytes[..])
        .take(limits.max_bytes as u64)
        .read_to_end(&mut raw)
    {
        Ok(_) => raw,
        Err(_) => bytes,
    };
    Some(nbt::read_named(&raw, limits).ok()?.0.tag)
}

fn read(path: &Path) -> Option<World> {
    scanned_world(path).ok().flatten().map(|s| s.world)
}

/// A folder with either level file is a world. Use the same recovery order as the game;
/// if neither is readable it is still a world, of unknown mods.
fn scanned_world(path: &Path) -> std::io::Result<Option<Scanned>> {
    let mut changed = None;
    let mut root = None;
    for file in ["level.dat", "level.dat_old"] {
        let dat = path.join(file);
        let meta = match fs::symlink_metadata(&dat) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        changed.get_or_insert_with(|| meta.modified().unwrap_or(UNIX_EPOCH));
        if dat.is_file() {
            root = level_dat(&dat).filter(|r| matches!(r.get("Data"), Some(Tag::Compound(_))));
            if root.is_some() {
                break;
            }
        }
    }
    let Some(changed) = changed else {
        return Ok(None);
    };
    let folder = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let known = root.is_some();
    let data = root.as_ref().and_then(|r| r.get("Data"));
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
        (true, _) => "Hardcore",
        (_, 1) => "Creative",
        (_, 2) => "Adventure",
        (_, 3) => "Spectator",
        _ => "Survival",
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
    let mut mods = listed(
        root.as_ref()
            .and_then(|r| r.get("fml"))
            .and_then(|fml| fml.get("LoadingModList")),
    );
    mods.extend(listed(
        root.as_ref()
            .and_then(|r| r.get("sunrise"))
            .and_then(|sunrise| sunrise.get("ContentModList")),
    ));
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

/// What a change that removes or drops mods asks first: which of them saved worlds use, and
/// which worlds. The answer is [`Answer::Stay`] unless the player says otherwise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub mods: Vec<String>,
    /// The worlds' names; one whose list of mods can't be read says so.
    pub worlds: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Answer {
    /// Leave the version, and the mods, as they are.
    #[default]
    Stay,
    /// Go on: what the mods added to those worlds is lost.
    Switch,
}

/// The question for removing the mods `ids` from `home`'s profile; None when no saved world uses
/// them, and the change goes ahead. Worlds that can't be read count as using every mod.
pub fn question(home: &Path, ids: &[String]) -> Result<Option<Question>, String> {
    let at_risk = at_risk(home, ids)?;
    if at_risk.is_empty() {
        return Ok(None);
    }
    let mut mods: Vec<String> = at_risk
        .iter()
        .flat_map(|(_, used, _)| used.clone())
        .collect();
    mods.sort();
    mods.dedup();
    let worlds = at_risk
        .iter()
        .map(|(world, _, known)| match known {
            true => world.name.clone(),
            false => crate::i18n::trf(
                "{name} (its list of mods can't be read)",
                &[("name", &world.name)],
            ),
        })
        .collect();
    Ok(Some(Question { mods, worlds }))
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

/// The world in `home` a command names: its folder, or its name (any case).
pub fn find(home: &Path, query: &str) -> Result<World, String> {
    let all = list(home);
    all.iter()
        .find(|w| w.folder == query)
        .or_else(|| all.iter().find(|w| w.name.eq_ignore_ascii_case(query)))
        .cloned()
        .ok_or_else(|| format!("No world {query} in {}", home.join("saves").display()))
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
        .unwrap()
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
pub fn copy(world: &World) -> Result<World, String> {
    let _lock = lock_world(world)?;
    let saves = world.path.parent().ok_or("A world folder has a parent")?;
    let folder = free_name(saves, &world.name, "", 1);
    let to = saves.join(&folder);
    let made = copy_dir(&world.path, &to).and_then(|()| rename_level(&to, &folder));
    if let Err(e) = made {
        let _ = fs::remove_dir_all(&to);
        return Err(format!("Could not copy {}: {e}", world.name));
    }
    read(&to).ok_or_else(|| format!("{} has no level.dat", to.display()))
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else if e.file_name() != LOCK {
            fs::copy(e.path(), target)?;
        }
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
pub fn delete(world: &World) -> Result<(), String> {
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
#[path = "worlds/tests.rs"]
mod tests;
