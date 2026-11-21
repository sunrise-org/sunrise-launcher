//! Permanent names for movable profile homes. Never put their owner inside the directory
//! it protects: Windows must be able to detach the whole directory while ownership is held.
use super::{creation_parent, Meta, OWN};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) const GATES: &str = ".launcher-instance-gates";
pub(super) const RETIRED: &str = ".launcher-instance-trash";
pub(super) const INITIAL: &str = ".profile-creation.json";

pub(crate) fn changed() -> String {
    crate::i18n::tr(
        "The profile changed since you reviewed this. Nothing was changed; choose again.",
    )
    .into()
}

pub(super) fn initial_choices(home: &Path) -> Option<Meta> {
    let path = home.join(INITIAL);
    let meta = fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || crate::native_cache::linked(&meta) {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

pub(crate) fn owner_file(game: &Path, home: &Path) -> Result<Option<PathBuf>, String> {
    owner_path(game, home, true)
}

/// A virgin profile has neither a possible live owner nor a journal to settle. Do not add
/// lock/registration files to that profile just because the player asked to move their data.
pub(crate) fn needs_settlement(game: &Path, home: &Path) -> Result<bool, String> {
    for item in [
        owner_path(game, home, false)?.unwrap_or_else(|| home.join(".install-txn.lock")),
        home.join(".mod-browser.lock"),
        home.join("mod-browser"),
    ] {
        match fs::symlink_metadata(item) {
            Ok(_) => return Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(false)
}

fn owner_path(game: &Path, home: &Path, create: bool) -> Result<Option<PathBuf>, String> {
    let invalid = || crate::i18n::tr("The profile folder couldn't be verified.").to_owned();
    let root = game.canonicalize().map_err(|error| error.to_string())?;
    let path = home.canonicalize().map_err(|error| error.to_string())?;
    let parts = path
        .strip_prefix(&root)
        .map_err(|_| invalid())?
        .iter()
        .collect::<Vec<_>>();
    let path = |kind, id: &std::ffi::OsStr| {
        if create {
            gate_path(&root, kind, id)
        } else {
            Ok(root.join(GATES).join(kind).join(id))
        }
    };
    match parts.as_slice() {
        [] => Ok(None),
        [collection, id] if *collection == OWN => path(OWN, id).map(Some),
        [collection, id, leaf] if *collection == "sunrise-packs" && *leaf == "home" => {
            path("sunrise-packs", id).map(Some)
        }
        _ => Err(invalid()),
    }
}

pub(crate) fn profile_paths(game: &Path) -> Result<Vec<PathBuf>, String> {
    let folder = |path: &Path| match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !crate::native_cache::linked(&meta) => Ok(true),
        Ok(meta) if meta.is_file() && !crate::native_cache::linked(&meta) => Ok(false),
        Ok(_) => Err(crate::i18n::tr("The profile folder couldn't be verified.").to_owned()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    };
    if !folder(game)? {
        return Ok(Vec::new());
    }
    let mut homes = vec![game.to_owned()];
    for (collection, pack) in [(OWN, false), ("sunrise-packs", true)] {
        let collection = game.join(collection);
        if !folder(&collection)? {
            continue;
        }
        for entry in fs::read_dir(&collection).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?.path();
            if folder(&entry)? {
                let home = if pack {
                    super::registry_pack::home(&entry)
                } else {
                    entry
                };
                if !pack || folder(&home)? {
                    homes.push(home);
                }
            }
        }
    }
    Ok(homes)
}

pub(super) fn gate_path(root: &Path, kind: &str, id: &std::ffi::OsStr) -> Result<PathBuf, String> {
    let id = id
        .to_str()
        .filter(|id| crate::browser::safe_name(id) == Some(*id))
        .ok_or_else(|| crate::i18n::tr("The profile folder couldn't be verified.").to_owned())?;
    let gates = creation_parent(root, GATES).map_err(|error| error.to_string())?;
    let folder = creation_parent(&gates, kind).map_err(|error| error.to_string())?;
    Ok(folder.join(id))
}
