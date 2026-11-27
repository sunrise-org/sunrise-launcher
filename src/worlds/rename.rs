//! Changing a world's name: `LevelName` in its level.dat, which the game's list shows. The
//! world's folder keeps its name, so nothing that points at it breaks.
use super::*;
use crate::i18n::{tr, trf};

/// Gives the world `query` the name `name` in its level.dat. A world the game has open is
/// refused, as its lock is held.
pub fn rename(
    instance: &crate::instances::Instance,
    query: &str,
    name: &str,
) -> Result<World, String> {
    let profile = crate::managed_play::Profile::open_selected(instance)?;
    rename_in(profile.home(), query, name)
}

pub(crate) fn rename_in(home: &Path, query: &str, name: &str) -> Result<World, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(tr("A world needs a name.").into());
    }
    let world = find(home, query)?;
    let failed = |e: &dyn std::fmt::Display| {
        trf(
            "Could not rename {world}: {error}",
            &[("world", &world.name), ("error", &e.to_string())],
        )
    };
    let _lock = lock_world(&world)?;
    let mut root = level_dat(&world.path.join("level.dat"))
        .ok_or_else(|| failed(&io::Error::from(io::ErrorKind::InvalidData)))?;
    let Some(Tag::Compound(data)) = root.get_mut("Data") else {
        return Err(failed(&io::Error::from(io::ErrorKind::InvalidData)));
    };
    data.insert("LevelName".into(), Tag::String(name.into()));
    save_level_dat(&world.path, root).map_err(|e| failed(&e))?;
    read(&world.path).ok_or_else(|| failed(&io::Error::from(io::ErrorKind::InvalidData)))
}

#[cfg(test)]
mod tests;
