//! How each launcher keeps its profiles: where the list is, and where a profile's name, game
//! version, play time and game folder are written. Every reader is best effort: a profile it
//! can't make sense of is left out, and nothing here panics on a file that isn't what it
//! expects.
use super::*;
use serde_json::Value;

/// The most of a launcher's file that is read: its settings are far smaller.
const MAX_FILE: u64 = 8 << 20;
/// The most profiles of one launcher, or entries counted in one folder.
const MAX_ENTRIES: usize = 5000;

/// The official launcher's name, as `sources` spells it and as it is shown.
const OFFICIAL: &str = "the official launcher";
const OFFICIAL_NAME: &str = "Minecraft Launcher";

/// The profiles of `launcher` whose files are in `root`.
pub(super) fn read_launcher(launcher: &str, root: &Path) -> Vec<Foreign> {
    match launcher {
        "Prism Launcher" | "MultiMC" => {
            profiles(&root.join("instances"), |dir| prism(launcher, dir))
        }
        "Modrinth App" => profiles(&root.join("profiles"), modrinth),
        "ATLauncher" => profiles(&root.join("instances"), atlauncher),
        "GDLauncher" => ["instances", "data/instances"]
            .iter()
            .flat_map(|dir| profiles(&root.join(dir), gdlauncher))
            .collect(),
        "CurseForge" => profiles(&root.join("Instances"), curseforge),
        OFFICIAL => flat(OFFICIAL_NAME, root).into_iter().collect(),
        other => flat(other, root).into_iter().collect(),
    }
}

/// A game folder picked by hand: a profile of a launcher it is recognised as, or the game
/// folder itself.
pub(super) fn read_folder(folder: &Path) -> Option<Foreign> {
    if !is_real_dir(folder) {
        return None;
    }
    let found = if folder.join("instance.cfg").is_file() {
        prism("Prism Launcher", folder)
    } else if folder.join("minecraftinstance.json").is_file() {
        curseforge(folder)
    } else if folder.join("profile.json").is_file() {
        modrinth(folder)
    } else if folder.join("instance.json").is_file() {
        atlauncher(folder)
    } else if folder.join("config.json").is_file() {
        gdlauncher(folder)
    } else {
        None
    };
    found
        .or_else(|| {
            // A launcher's root, with the game's files one folder down.
            [".minecraft", "minecraft"]
                .iter()
                .map(|d| folder.join(d))
                .find(|d| is_real_dir(d) && Contents::read(d).any())
                .and_then(|game| other_folder(folder, &game))
        })
        .or_else(|| other_folder(folder, folder))
        .filter(|f| f.contents.any())
}

/// A folder picked by hand that no launcher claims, named for the folder.
fn other_folder(picked: &Path, game: &Path) -> Option<Foreign> {
    let name = picked.file_name()?.to_string_lossy().into_owned();
    Some(entry(
        crate::i18n::tr("Another folder"),
        name,
        game.to_owned(),
    ))
}

/// A profile found in `dir`'s subfolders, each read by `read`.
fn profiles(dir: &Path, read: impl Fn(&Path) -> Option<Foreign>) -> Vec<Foreign> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .take(MAX_ENTRIES)
        .map(|e| e.path())
        .filter(|p| is_real_dir(p))
        .filter_map(|p| read(&p))
        .collect()
}

/// A profile's entry with its folder's contents counted.
fn entry(launcher: &str, name: String, game: PathBuf) -> Foreign {
    Foreign {
        launcher: launcher.to_owned(),
        name,
        contents: Contents::read(&game),
        folder: game,
        version: None,
        last_played: None,
        played_secs: None,
    }
}

/// A launcher whose root is a game folder itself (the official launcher, Dawn Client). Left
/// out when it holds nothing to import.
fn flat(launcher: &str, root: &Path) -> Option<Foreign> {
    let found = entry(launcher, launcher.to_owned(), root.to_owned());
    found.contents.any().then_some(found)
}

/// Prism Launcher and MultiMC: `instance.cfg` (name, play time) beside the game folder, and
/// the game's version in `mmc-pack.json`.
fn prism(launcher: &str, dir: &Path) -> Option<Foreign> {
    let cfg = ini(&dir.join("instance.cfg"))?;
    let game = [".minecraft", "minecraft"]
        .iter()
        .map(|d| dir.join(d))
        .find(|d| is_real_dir(d))?;
    let mut found = entry(launcher, name_or_folder(cfg.get("name"), dir)?, game);
    found.played_secs = cfg.get("totalTimePlayed").and_then(|v| v.parse().ok());
    found.last_played = cfg
        .get("lastLaunchTime")
        .and_then(|v| v.parse::<u64>().ok())
        .map(|ms| ms / 1000)
        .filter(|s| *s > 0);
    found.version = json(&dir.join("mmc-pack.json"))
        .and_then(|pack| {
            pack["components"]
                .as_array()?
                .iter()
                .find(|c| c["uid"] == "net.minecraft")
                .and_then(|c| c["version"].as_str().map(str::to_owned))
        })
        .filter(|v| !v.is_empty());
    Some(found)
}

/// The Modrinth App: a profile's folder is its game folder; `profile.json` (older versions
/// of the app, which kept it there) names it and counts the time played.
fn modrinth(dir: &Path) -> Option<Foreign> {
    let profile = json(&dir.join("profile.json"));
    let mut found = entry("Modrinth App", folder_name(dir)?, dir.to_owned());
    if profile.is_none() && !found.contents.any() {
        return None;
    }
    if let Some(profile) = profile {
        // Some versions keep these under `metadata`.
        let at = |key: &str| {
            [&profile[key], &profile["metadata"][key]]
                .into_iter()
                .find(|v| !v.is_null())
                .cloned()
        };
        if let Some(name) = at("name").and_then(|v| v.as_str().map(str::to_owned)) {
            if !name.trim().is_empty() {
                found.name = name;
            }
        }
        found.version = at("game_version").and_then(|v| v.as_str().map(str::to_owned));
        let played: Vec<u64> = ["recent_time_played", "submitted_time_played"]
            .iter()
            .filter_map(|key| at(key).and_then(|v| v.as_u64()))
            .collect();
        found.played_secs = (!played.is_empty()).then(|| played.iter().sum());
        found.last_played = at("last_played").and_then(|v| seconds(&v));
    }
    Some(found)
}

/// ATLauncher: `instance.json` holds the name, the game version and the time played; the game
/// folder is the instance's own.
fn atlauncher(dir: &Path) -> Option<Foreign> {
    let instance = json(&dir.join("instance.json"))?;
    let launcher = &instance["launcher"];
    let at = |key: &str| {
        [&launcher[key], &instance[key]]
            .into_iter()
            .find(|v| !v.is_null())
            .cloned()
    };
    let name = at("name")
        .and_then(|v| v.as_str().map(str::to_owned))
        .filter(|n| !n.trim().is_empty())
        .or_else(|| folder_name(dir))?;
    let mut found = entry("ATLauncher", name, dir.to_owned());
    found.version = instance["id"]
        .as_str()
        .or(instance["minecraftVersion"].as_str())
        .map(str::to_owned);
    found.played_secs = at("totalPlayTime").and_then(|v| v.as_u64());
    found.last_played = at("lastPlayed").and_then(|v| seconds(&v));
    Some(found)
}

/// GDLauncher: the older one keeps `config.json` in the profile's folder with the game's files
/// in it; Carbon keeps them in its `instance` folder.
fn gdlauncher(dir: &Path) -> Option<Foreign> {
    let config = json(&dir.join("config.json"));
    let carbon = dir.join("instance");
    let game = match (&config, is_real_dir(&carbon)) {
        (Some(_), _) => dir.to_owned(),
        (None, true) => carbon,
        (None, false) => return None,
    };
    let mut found = entry("GDLauncher", folder_name(dir)?, game);
    if let Some(config) = config {
        found.version = config["loader"]["mcVersion"].as_str().map(str::to_owned);
        found.last_played = seconds(&config["lastPlayed"]);
    }
    Some(found)
}

/// CurseForge: `minecraftinstance.json` names the profile and its game version; the profile's
/// folder is the game folder.
fn curseforge(dir: &Path) -> Option<Foreign> {
    let profile = json(&dir.join("minecraftinstance.json"))?;
    let name = profile["name"]
        .as_str()
        .filter(|n| !n.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| folder_name(dir))?;
    let mut found = entry("CurseForge", name, dir.to_owned());
    found.version = profile["gameVersion"]
        .as_str()
        .or(profile["baseModLoader"]["minecraftVersion"].as_str())
        .filter(|v| !v.is_empty())
        .map(str::to_owned);
    found.last_played = seconds(&profile["lastPlayed"]);
    Some(found)
}

fn folder_name(dir: &Path) -> Option<String> {
    Some(dir.file_name()?.to_string_lossy().into_owned())
}

fn name_or_folder(name: Option<&String>, dir: &Path) -> Option<String> {
    match name.map(|n| n.trim()).filter(|n| !n.is_empty()) {
        Some(name) => Some(name.to_owned()),
        None => folder_name(dir),
    }
}

/// A real folder: not a link to somewhere else.
fn is_real_dir(path: &Path) -> bool {
    path.symlink_metadata()
        .is_ok_and(|m| m.is_dir() && !crate::native_cache::linked(&m))
}

/// The entries of `dir` that `keep` accepts and whose names don't start with a dot (editors'
/// and systems' leftovers), up to a limit.
pub(super) fn count_children(dir: &Path, keep: impl Fn(&Path) -> bool) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .take(MAX_ENTRIES)
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter(|e| keep(&e.path()))
        .count()
}

/// A JSON file of a modest size, or None when it is missing, too big or not JSON.
fn json(path: &Path) -> Option<Value> {
    let meta = path.symlink_metadata().ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// A `key=value` settings file (Qt's ini, as Prism Launcher and MultiMC write it).
fn ini(path: &Path) -> Option<std::collections::HashMap<String, String>> {
    let meta = path.symlink_metadata().ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE {
        return None;
    }
    let text = String::from_utf8_lossy(&fs::read(path).ok()?).into_owned();
    Some(
        text.lines()
            .filter_map(|line| line.split_once('='))
            .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
            .collect(),
    )
}

/// A moment as seconds since 1970 from what launchers write: milliseconds or seconds as a
/// number, or an RFC 3339 date.
fn seconds(value: &Value) -> Option<u64> {
    let secs = match value {
        Value::Number(n) => {
            let n = n.as_u64()?;
            // Milliseconds past 1973 are bigger than any seconds count this century.
            if n > 100_000_000_000 {
                n / 1000
            } else {
                n
            }
        }
        Value::String(text) => chrono::DateTime::parse_from_rfc3339(text)
            .ok()
            .and_then(|t| u64::try_from(t.timestamp()).ok())?,
        _ => return None,
    };
    (secs > 0).then_some(secs)
}
